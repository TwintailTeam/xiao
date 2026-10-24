import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Window
import QtWebEngine
import QtWebChannel
import Qt.labs.platform as Platform
import com.xiao.shell

Window {
    id: root
    readonly property var cfg: JSON.parse(backend.config())
    readonly property var win: (cfg.app && cfg.app.windows && cfg.app.windows.length) ? cfg.app.windows[0] : ({})

    width: win.width || 800
    height: win.height || 600
    minimumWidth: win.resizable === false ? width : (win.minWidth || 0)
    minimumHeight: win.resizable === false ? height : (win.minHeight || 0)
    maximumWidth: win.resizable === false ? width : (win.maxWidth || 16777215)
    maximumHeight: win.resizable === false ? height : (win.maxHeight || 16777215)
    title: win.title || cfg.productName || "app"
    color: win.backgroundColor || "#101014"
    visible: win.visible !== false
    flags: (win.decorations === false ? Qt.FramelessWindowHint : 0)
        | (win.alwaysOnTop === true ? Qt.WindowStaysOnTopHint : 0)
        | Qt.Window
    readonly property bool devMode: backend.isDev()
    property bool devtoolsOpen: false
    property var channelObjects: []

    Component.onCompleted: {
        if (win.x !== undefined && win.y !== undefined) {
            x = win.x
            y = win.y
        } else if (win.center === true && Screen) {
            x = Screen.virtualX + (Screen.width - width) / 2
            y = Screen.virtualY + (Screen.height - height) / 2
        }

        if (win.fullscreen === true)
            visibility = Window.FullScreen
        else if (win.maximized === true)
            visibility = Window.Maximized

        backend.notifyWindowEvent("scale-changed", JSON.stringify({ scaleFactor: root.scaleFactor }))
        backend.notifyWindowEvent("theme-changed", JSON.stringify({ theme: root.themeName() }))

        startBrowsing()

        if (devMode)
            console.log("[qml] " + title + " ready — F5 reload · F12 devtools · http://localhost:9222")
    }

    property bool preventClose: false
    property bool shuttingDown: false

    Backend {
        id: backend

        onWindowCommand: function(action, argsJson) {
            root.applyWindowCommand(action, JSON.parse(argsJson || "{}"))
        }

        onTrayCommand: function(action, argsJson) {
            root.applyTrayCommand(action, JSON.parse(argsJson || "{}"))
        }

        onDialogCommand: function(argsJson) {
            root.openDialog(JSON.parse(argsJson || "{}"))
        }

        onWebChannelCommand: function(action, argsJson) {
            root.applyWebChannelCommand(action, JSON.parse(argsJson || "{}"))
        }
    }

    Page {
        id: page
        WebChannel.id: "backend"
    }

    property var trayMenuModel: []
    property bool trayMenuOnLeftClick: true

    Platform.SystemTrayIcon {
        id: tray
        readonly property var trayCfg: (root.cfg.app && root.cfg.app.trayIcon) ? root.cfg.app.trayIcon : ({})
        visible: !!trayCfg.iconPath
        tooltip: trayCfg.tooltip || root.title
        icon.source: backend.trayIconUrl()

        menu: Platform.Menu {
            id: trayMenu
            visible: false

            Instantiator {
                model: root.trayMenuModel
                onObjectAdded: (index, object) => trayMenu.insertItem(index, object)
                onObjectRemoved: (index, object) => trayMenu.removeItem(object)

                delegate: Platform.MenuItem {
                    required property var modelData
                    text: modelData.text || ""
                    enabled: modelData.enabled !== false
                    separator: modelData.separator === true
                    checkable: modelData.checked !== undefined && modelData.checked !== null
                    checked: modelData.checked === true
                    onTriggered: backend.notifyTrayEvent("menu", JSON.stringify({ id: modelData.id, checked: checked }))
                }
            }
        }

        onActivated: function(reason) {
            if (reason === Platform.SystemTrayIcon.Trigger && root.trayMenuOnLeftClick && root.trayMenuModel.length) {
                switch (Qt.platform.pluginName) {
                case "xcb":
                case "windows":
                    trayMenu.open()
                    break
                case "cocoa":
                    break
                default:
                    root.warnLeftClickMenu()
                }
            }

            backend.notifyTrayEvent("activated", JSON.stringify({ reason: reason }))
        }

        onMessageClicked: backend.notifyTrayEvent("message-clicked", "{}")
    }

    property bool warnedLeftClickMenu: false

    function warnLeftClickMenu() {
        if (root.warnedLeftClickMenu)
            return
        root.warnedLeftClickMenu = true
        console.warn("[tray] opening the menu on left-click is the status bar's call on Wayland, the app cannot force it")
    }

    function applyTrayCommand(action, a) {
        switch (action) {
        case "show": root.trayOnlyForMessage = false; tray.visible = true; break
        case "hide": root.trayOnlyForMessage = false; tray.visible = false; break
        case "setTooltip": tray.tooltip = a.value; break
        case "setIcon": tray.icon.source = a.value || ""; break
        case "setMenu": root.trayMenuModel = a.items || []; break
        case "setMenuOnLeftClick": root.trayMenuOnLeftClick = a.value !== false; break
        case "updateItem": {
            const items = root.trayMenuModel.slice()
            for (let i = 0; i < items.length; i++) {
                if (items[i].id !== a.id)
                    continue

                const item = Object.assign({}, items[i])
                if (a.text !== undefined)    item.text = a.text
                if (a.enabled !== undefined) item.enabled = a.enabled
                if (a.checked !== undefined) item.checked = a.checked
                items[i] = item
                root.trayMenuModel = items
                return
            }
            console.warn("[tray] no menu item with id: " + a.id)
            break
        }
        case "showMessage":
            if (backend.notify(a.title || "", a.message || ""))
                break
            if (!tray.visible) {
                root.trayOnlyForMessage = true
                tray.visible = true
            }
            tray.showMessage(a.title || "", a.message || "")
            if (root.trayOnlyForMessage)
                trayMessageDone.restart()
            break
        default:
            console.warn("[tray] unknown command: " + action)
        }
    }

    property bool trayOnlyForMessage: false

    Timer {
        id: trayMessageDone
        interval: 10000
        onTriggered: {
            if (root.trayOnlyForMessage) {
                root.trayOnlyForMessage = false
                tray.visible = false
            }
        }
    }

    property string pendingDialogId: ""
    property var dialogQueue: []

    function reportDialog(payload) {
        if (!root.pendingDialogId)
            return
        const id = root.pendingDialogId
        root.pendingDialogId = ""
        backend.notifyDialogResult(id, JSON.stringify(payload))

        if (root.dialogQueue.length) {
            const next = root.dialogQueue[0]
            root.dialogQueue = root.dialogQueue.slice(1)
            Qt.callLater(root.openDialog, next)
        }
    }

    function urlToPath(url) {
        let path = decodeURIComponent(url.toString().replace(/^file:\/\//, ""))
        return /^\/[A-Za-z]:/.test(path) ? path.substring(1) : path
    }

    function pathToUrl(path) {
        if (!path)
            return ""
        const slashed = path.replace(/\\/g, "/")
        return slashed.charAt(0) === "/" ? "file://" + slashed : "file:///" + slashed
    }

    function urlsToPaths(urls) {
        const out = []
        for (let i = 0; i < urls.length; i++)
            out.push(root.urlToPath(urls[i]))
        return out
    }

    function openDialog(a) {
        if (root.pendingDialogId) {
            root.dialogQueue = root.dialogQueue.concat([a])
            return
        }
        root.pendingDialogId = a.id || ""

        dialogs.active = true
        const fileDialog = dialogs.item.fileDialog
        const folderDialog = dialogs.item.folderDialog

        switch (a.kind) {
        case "open":
            fileDialog.fileMode = Platform.FileDialog.OpenFiles
            fileDialog.title = a.title || qsTr("Open")
            fileDialog.nameFilters = a.filters || ["All files (*)"]
            if (a.defaultPath) fileDialog.folder = root.pathToUrl(a.defaultPath)
            fileDialog.open()
            break
        case "save":
            fileDialog.fileMode = Platform.FileDialog.SaveFile
            fileDialog.title = a.title || qsTr("Save")
            fileDialog.nameFilters = a.filters || ["All files (*)"]
            if (a.defaultPath) fileDialog.currentFile = root.pathToUrl(a.defaultPath)
            fileDialog.open()
            break
        case "folder":
            folderDialog.title = a.title || qsTr("Select folder")
            if (a.defaultPath) folderDialog.folder = root.pathToUrl(a.defaultPath)
            folderDialog.open()
            break
        case "message":
            messageDialog.createObject(root, {
                title: a.title || root.title,
                text: a.text || "",
                informativeText: a.detail || "",
                buttons: a.buttons === "yesno"
                    ? (Platform.MessageDialog.Yes | Platform.MessageDialog.No)
                    : a.buttons === "okcancel"
                        ? (Platform.MessageDialog.Ok | Platform.MessageDialog.Cancel)
                        : Platform.MessageDialog.Ok,
            }).open()
            break
        default:
            root.reportDialog({ cancelled: true, error: "unknown dialog kind: " + a.kind })
        }
    }

    Loader {
        id: dialogs
        active: false
        sourceComponent: Item {
            readonly property alias fileDialog: fileDialog
            readonly property alias folderDialog: folderDialog

            Platform.FileDialog {
                id: fileDialog
                onAccepted: root.reportDialog({
                    cancelled: false,
                    paths: root.urlsToPaths(fileMode === Platform.FileDialog.SaveFile ? [currentFile] : files),
                })
                onRejected: root.reportDialog({ cancelled: true, paths: [] })
            }

            Platform.FolderDialog {
                id: folderDialog
                onAccepted: root.reportDialog({ cancelled: false, paths: root.urlsToPaths([folder]) })
                onRejected: root.reportDialog({ cancelled: true, paths: [] })
            }

        }
    }

    Component {
        id: messageDialog

        Platform.MessageDialog {
            property bool answered: false

            function answer(payload) {
                if (answered)
                    return
                answered = true
                root.reportDialog(payload)
                destroy()
            }

            onOkClicked:     answer({ cancelled: false, button: "ok" })
            onYesClicked:    answer({ cancelled: false, button: "yes" })
            onNoClicked:     answer({ cancelled: false, button: "no" })
            onCancelClicked: answer({ cancelled: true, button: "cancel" })
            onRejected:      answer({ cancelled: true, button: "cancel" })
        }
    }

    function applyWindowCommand(action, a) {
        switch (action) {
        case "show":
            root.visibility === Window.Minimized ? root.showNormal() : root.show()
            root.raise()
            root.requestActivate()
            break
        case "hide":            root.hide(); break
        case "close":           root.close(); break
        case "destroy":         root.shutdown(); break
        case "minimize":        root.showMinimized(); break
        case "maximize":        root.showMaximized(); break
        case "unmaximize":      root.showNormal(); break
        case "toggleMaximize":
            root.visibility === Window.Maximized ? root.showNormal() : root.showMaximized()
            break
        case "setFullscreen":   a.value ? root.showFullScreen() : root.showNormal(); break
        case "center":
            root.x = Math.round(Screen.virtualX + (Screen.width - root.width) / 2)
            root.y = Math.round(Screen.virtualY + (Screen.height - root.height) / 2)
            break
        case "focus":           root.raise(); root.requestActivate(); break
        case "setTitle":        root.title = a.value; break
        case "setSize":         root.width = a.width; root.height = a.height; break
        case "setMinSize":      root.minimumWidth = a.width; root.minimumHeight = a.height; break
        case "setPosition":     root.x = a.x; root.y = a.y; break
        case "setResizable":
            if (a.value) {
                root.maximumWidth = 16777215
                root.maximumHeight = 16777215
            } else {
                root.maximumWidth = root.width
                root.maximumHeight = root.height
                root.minimumWidth = root.width
                root.minimumHeight = root.height
            }
            break
        case "setAlwaysOnTop":
            root.flags = a.value ? (root.flags | Qt.WindowStaysOnTopHint)
                                 : (root.flags & ~Qt.WindowStaysOnTopHint)
            break
        case "setPreventClose": root.preventClose = a.value; break
        default:
            console.warn("[window] unknown command: " + action)
        }
    }

    onClosing: function(close) {
        if (root.shuttingDown)
            return

        backend.notifyWindowEvent("close-requested", JSON.stringify({ prevented: root.preventClose }))

        if (root.preventClose) {
            close.accepted = false
            return
        }

        close.accepted = false
        root.shutdown()
    }

    function shutdown() {
        if (root.shuttingDown)
            return
        root.shuttingDown = true

        backend.notifyWindowEvent("destroyed", "{}")
        quitDeferred.start()
    }

    Timer {
        id: quitDeferred
        interval: 0
        onTriggered: Qt.quit()
    }

    onActiveChanged: backend.notifyWindowEvent(active ? "focus" : "blur", "{}")

    onVisibilityChanged: {
        backend.notifyWindowEvent("visibility", JSON.stringify({ visibility: root.visibility }))
        Qt.callLater(root.applyHiddenLifecycle)
    }

    readonly property string whenHidden: win.whenHidden || "keep"

    function applyHiddenLifecycle() {
        if (root.whenHidden !== "freeze" && root.whenHidden !== "discard")
            return

        if (root.visibility === Window.Hidden) {
            view.lifecycleState = WebEngineView.LifecycleState.Frozen
            if (root.whenHidden === "discard")
                view.lifecycleState = WebEngineView.LifecycleState.Discarded
        } else if (view.lifecycleState !== WebEngineView.LifecycleState.Active) {
            view.lifecycleState = WebEngineView.LifecycleState.Active
        }
    }

    Timer {
        id: resizeSettled
        interval: 120
        onTriggered: backend.notifyWindowEvent("resized", JSON.stringify({ width: root.width, height: root.height }))
    }

    onWidthChanged: resizeSettled.restart()
    onHeightChanged: resizeSettled.restart()

    Timer {
        id: moveSettled
        interval: 120
        onTriggered: backend.notifyWindowEvent("moved", JSON.stringify({ x: root.x, y: root.y }))
    }

    onXChanged: moveSettled.restart()
    onYChanged: moveSettled.restart()

    readonly property real scaleFactor: Screen.devicePixelRatio

    onScaleFactorChanged: backend.notifyWindowEvent("scale-changed", JSON.stringify({ scaleFactor: root.scaleFactor }))

    readonly property int colorScheme: Application.styleHints.colorScheme

    onColorSchemeChanged: backend.notifyWindowEvent("theme-changed", JSON.stringify({ theme: root.themeName() }))

    function themeName() {
        switch (root.colorScheme) {
        case Qt.ColorScheme.Light: return "light"
        case Qt.ColorScheme.Dark: return "dark"
        default: return "unknown"
        }
    }

    WebChannel {
        id: channel
        registeredObjects: [page].concat(root.channelObjects)
    }

    readonly property var storage: JSON.parse(backend.webStorage())

    WebEngineProfilePrototype {
        id: profilePrototype
        storageName: root.storage.storageName
        persistentStoragePath: root.storage.storagePath
        cachePath: root.storage.cachePath
        httpCacheType: WebEngineProfile.DiskHttpCache
        persistentCookiesPolicy: WebEngineProfile.ForcePersistentCookies
    }

    property var appProfile: null

    function startBrowsing() {
        root.appProfile = profilePrototype.instance()
        root.appProfile.downloadPath = root.storage.downloadPath
        view.profile = root.appProfile

        if (backend.injectsScripts()) {
            const scripts = root.initialScripts()
            for (let i = 0; i < scripts.length; i++)
                view.userScripts.insert([scripts[i]])
        }

        view.url = backend.startUrl()
    }

    function initialScripts() {
        function script(name, source) {
            return {
                name: name,
                sourceCode: source,
                injectionPoint: WebEngineScript.DocumentCreation,
                worldId: WebEngineScript.MainWorld,
                runsOnSubFrames: false,
            }
        }

        const scripts = []

        const csp = backend.cspScript()
        if (csp)
            scripts.push(script("xiao-csp", csp))

        scripts.push({
            name: "qwebchannel",
            sourceUrl: "qrc:///qtwebchannel/qwebchannel.js",
            injectionPoint: WebEngineScript.DocumentCreation,
            worldId: WebEngineScript.MainWorld,
            runsOnSubFrames: false,
        })

        const extra = JSON.parse(backend.userScripts())
        for (let i = 0; i < extra.length; i++)
            scripts.push(script(extra[i].name, extra[i].source))

        return scripts
    }

    function applyWebChannelCommand(action, a) {
        const scripts = view.userScripts

        switch (action) {
        case "eval":
            if (a.id)
                view.runJavaScript(a.script, function(result) {
                    backend.notifyWebChannelResult(a.id, JSON.stringify(result === undefined ? null : result))
                })
            else
                view.runJavaScript(a.script)
            break
        case "inject":
            scripts.insert([{
                name: a.name,
                sourceCode: a.source || "",
                injectionPoint: WebEngineScript.DocumentCreation,
                worldId: WebEngineScript.MainWorld,
                runsOnSubFrames: false,
            }])
            break
        case "removeScript": {
            const found = scripts.find(a.name)
            for (let i = 0; i < found.length; i++)
                scripts.remove(found[i])
            break
        }
        default:
            console.warn("[webchannel] unknown command: " + action)
        }
    }

    WebEngineView {
        id: view

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: root.devtoolsOpen ? parent.height * 0.62 : parent.height
        webChannel: channel
        backgroundColor: root.color
        devToolsView: devtoolsLoader.item

        Behavior on height {
            NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
        }

        onContextMenuRequested: function(request) {
            request.accepted = true
        }

        onJavaScriptConsoleMessage: function(level, message, lineNumber, sourceID) {
            if (root.devMode)
                console.log("[web] " + message + "  (" + sourceID + ":" + lineNumber + ")")
        }

        onLoadingChanged: function(request) {
            if (request.status === WebEngineView.LoadStartedStatus) {
                if (backend.isAppUrl(request.url.toString()))
                    backend.pageLeaving()
            } else if (request.status === WebEngineView.LoadFailedStatus) {
                console.warn("[web] cannot load " + request.url)
                if (root.devMode)
                    reconnect.start()
            } else if (request.status === WebEngineView.LoadSucceededStatus) {
                reconnect.stop()
                if (backend.isAppUrl(request.url.toString()))
                    view.runJavaScript(root.inputWatch, WebEngineScript.ApplicationWorld)
            }
        }

        onNavigationRequested: function(request) {
            const url = request.url.toString()
            if (!request.isMainFrame || backend.isAppUrl(url))
                return

            request.reject()
            if (request.navigationType !== WebEngineNavigationRequest.LinkClickedNavigation) {
                console.warn("[navigation] refused " + url)
                return
            }
            view.runJavaScript(root.inputRecent, WebEngineScript.ApplicationWorld, function(byUser) {
                if (byUser)
                    backend.openExternal(url)
                else
                    console.warn("[navigation] refused " + url + ": not started by the user")
            })
        }

        onNewWindowRequested: function(request) {
            const url = request.requestedUrl.toString()
            if (backend.isAppUrl(url))
                view.url = url
            else if (request.userInitiated)
                backend.openExternal(url)
            else
                console.warn("[navigation] refused a window for " + url + ": not opened by the user")
        }

        onRenderProcessTerminated: function(terminationStatus, exitCode) {
            if (terminationStatus === WebEngineView.NormalTerminationStatus)
                return

            backend.notifyWindowEvent("renderer-terminated", JSON.stringify({ status: terminationStatus, exitCode: exitCode }))
            const now = Date.now()
            root.rendererCrashes = root.rendererCrashes.filter(function(at) { return now - at < 30000 }).concat([now])
            if (root.rendererCrashes.length <= 3) {
                console.warn("[web] renderer terminated (status " + terminationStatus + ", exit " + exitCode + "), reloading")
                rendererRestart.start()
            } else {
                console.warn("[web] renderer terminated again, not reloading after 3 crashes in 30 s")
                root.rendererGaveUp = true
            }
        }
    }

    property var rendererCrashes: []
    property bool rendererGaveUp: false

    Rectangle {
        id: crashScreen
        anchors.fill: view
        visible: root.rendererGaveUp
        color: root.color
        z: 1

        readonly property color textColor: root.color.hslLightness > 0.5 ? "#1f1f1f" : "#f2f2f2"

        Column {
            anchors.centerIn: parent
            width: Math.min(parent.width - 48, 420)
            spacing: 12

            Text {
                width: parent.width
                text: qsTr("Something went wrong")
                color: crashScreen.textColor
                font.pixelSize: 20
                font.bold: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }

            Text {
                width: parent.width
                text: qsTr("The page crashed and couldn't be loaded again. Reload it, or restart the app.")
                color: crashScreen.textColor
                opacity: 0.75
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 8

                Controls.Button {
                    text: qsTr("Reload")
                    onClicked: {
                        root.rendererCrashes = []
                        root.rendererGaveUp = false
                        view.reload()
                    }
                }

                Controls.Button {
                    text: qsTr("Restart")
                    highlighted: true
                    onClicked: backend.restartApp()
                }
            }
        }
    }

    readonly property string inputWatch: "(function () {"
        + "  if (window.__xiaoInput) return;"
        + "  window.__xiaoInput = { at: -Infinity };"
        + "  ['pointerdown', 'pointerup', 'click', 'keydown'].forEach(function (type) {"
        + "    addEventListener(type, function (e) { if (e.isTrusted) window.__xiaoInput.at = performance.now(); }, true);"
        + "  });"
        + "})()"
    readonly property string inputRecent: "!!window.__xiaoInput && performance.now() - window.__xiaoInput.at < 2000"

    Timer {
        id: rendererRestart
        interval: 500
        onTriggered: view.reload()
    }

    Timer {
        id: reconnect
        interval: 700
        repeat: true
        onTriggered: {
            console.log("[dev] dev server not up yet, retrying…")
            view.reload()
        }
    }

    Loader {
        id: devtoolsLoader
        active: root.devMode && root.devtoolsOpen
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: view.bottom
        anchors.bottom: parent.bottom
        sourceComponent: Component {
            WebEngineView {
                onLoadingChanged: function(request) {
                    if (request.status === WebEngineView.LoadSucceededStatus)
                        console.log("[dev] inspector loaded")
                    else if (request.status === WebEngineView.LoadFailedStatus)
                        console.warn("[dev] inspector cannot load " + request.url)
                }

                onWindowCloseRequested: {
                    root.devtoolsOpen = false
                    console.log("[dev] devtools closed")
                }
            }
        }
    }

    Loader {
        active: root.devMode

        sourceComponent: Component {
            Item {
                Shortcut {
                    sequences: ["F5", "Ctrl+R"]
                    onActivated: {
                        console.log("[dev] reload")
                        view.reload()
                    }
                }

                Shortcut {
                    sequence: "Ctrl+Shift+R"
                    onActivated: {
                        console.log("[dev] hard reload (cache bypassed)")
                        view.triggerWebAction(WebEngineView.ReloadAndBypassCache)
                    }
                }

                Shortcut {
                    sequences: ["F12", "Ctrl+Shift+I"]
                    onActivated: {
                        root.devtoolsOpen = !root.devtoolsOpen
                        console.log("[dev] devtools " + (root.devtoolsOpen ? "opened" : "closed"))
                    }
                }

                Rectangle {
                    parent: root.contentItem
                    visible: !root.devtoolsOpen
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 10
                    radius: 6
                    color: "#1e293bcc"
                    border.color: "#334155"
                    width: hint.implicitWidth + 16
                    height: hint.implicitHeight + 10

                    Text {
                        id: hint
                        anchors.centerIn: parent
                        text: "F5 reload · F12 devtools"
                        color: "#64748b"
                        font.pixelSize: 11
                    }
                }
            }
        }
    }
}
