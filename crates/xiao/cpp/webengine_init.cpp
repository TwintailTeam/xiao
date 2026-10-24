#include <QtWebEngineQuick/QtWebEngineQuick>
#include <QtWebEngineQuick/QQuickWebEngineProfile>
#include <QtWebEngineCore/QWebEngineUrlRequestJob>
#include <QtWebEngineCore/QWebEngineUrlScheme>
#include <QtWebEngineCore/QWebEngineUrlSchemeHandler>
#include <QtCore/QBuffer>
#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonValue>
#include <QtCore/QMimeDatabase>
#include <QtCore/QMultiMap>
#include <QtCore/QSettings>
#include <QtCore/QStandardPaths>
#include <QtGui/QFileOpenEvent>
#include <QtCore/QByteArray>
#include <QtCore/QCoreApplication>
#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QFileInfo>
#include <QtCore/QMimeData>
#include <QtCore/QPointer>
#include <QtCore/QString>
#include <QtCore/QThread>
#include <QtCore/QTimer>
#include <QtCore/QUrl>
#include <QtGui/QWindow>
#include <QtQuick/QQuickWindow>
#include <QtGui/QClipboard>
#include <QtGui/QDesktopServices>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QPixmap>
#include <QtNetwork/QLocalServer>
#include <QtNetwork/QLocalSocket>
#include <QtQml/QQmlApplicationEngine>
#include <QtWidgets/QApplication>
#include <new>
#include <vector>
#ifdef Q_OS_LINUX
#include <cstdint>
#include <cstring>
#include <dlfcn.h>
#include <QtCore/QStringList>
#include <QtCore/QVariantMap>
#include <QtDBus/QDBusConnection>
#include <QtDBus/QDBusInterface>
#include <QtDBus/QDBusMessage>
#include <QtDBus/QDBusReply>
#endif
#ifdef Q_OS_WIN
#include <objbase.h>
#include <shlobj.h>
#include <windows.h>
#include <propsys.h>
#include <shellapi.h>
#include <cstring>
#include <utility>
#endif
#ifdef Q_OS_MACOS
extern "C" bool reveal_path_mac(const char* path);
#endif

static const char kFrontendScheme[] = "xiao";

static_assert(sizeof(QString) == 3 * sizeof(void*), "QString must be three pointers wide");

#ifdef Q_OS_LINUX
namespace {

using EGLBoolean = unsigned int;
using EGLenum = unsigned int;
using EGLint = int32_t;
using EGLAttrib = intptr_t;
using EGLDisplay = void*;
using EGLDeviceEXT = void*;

constexpr EGLint kEglExtensions = 0x3055;
constexpr EGLenum kEglPlatformWayland = 0x31D8;
constexpr EGLint kEglDevice = 0x322C;
constexpr EGLint kEglDrmRenderNodeFile = 0x3377;

bool has_extension(const char* list, const char* name) {
    if (!list) {
        return false;
    }
    const size_t len = strlen(name);
    for (const char* at = strstr(list, name); at; at = strstr(at + 1, name)) {
        const bool starts = at == list || at[-1] == ' ';
        const bool ends = at[len] == '\0' || at[len] == ' ';
        if (starts && ends) {
            return true;
        }
    }
    return false;
}

template <typename T>
T symbol(void* library, const char* name) {
    return reinterpret_cast<T>(dlsym(library, name));
}

}
#endif

extern "C" char* compositor_render_node() {
#ifdef Q_OS_LINUX
    void* wayland = dlopen("libwayland-client.so.0", RTLD_NOW | RTLD_LOCAL);
    void* egl = wayland ? dlopen("libEGL.so.1", RTLD_NOW | RTLD_LOCAL) : nullptr;
    if (!egl) {
        return nullptr;
    }

    auto connect = symbol<void* (*)(const char*)>(wayland, "wl_display_connect");
    auto disconnect = symbol<void (*)(void*)>(wayland, "wl_display_disconnect");
    auto getProcAddress = symbol<void* (*)(const char*)>(egl, "eglGetProcAddress");
    auto queryString = symbol<const char* (*)(EGLDisplay, EGLint)>(egl, "eglQueryString");
    auto initialize = symbol<EGLBoolean (*)(EGLDisplay, EGLint*, EGLint*)>(egl, "eglInitialize");
    auto terminate = symbol<EGLBoolean (*)(EGLDisplay)>(egl, "eglTerminate");
    if (!connect || !disconnect || !getProcAddress || !queryString || !initialize || !terminate) {
        return nullptr;
    }

    const char* clientExtensions = queryString(nullptr, kEglExtensions);
    const bool platform = has_extension(clientExtensions, "EGL_KHR_platform_wayland") || has_extension(clientExtensions, "EGL_EXT_platform_wayland");
    const bool deviceQuery = has_extension(clientExtensions, "EGL_EXT_device_query") || has_extension(clientExtensions, "EGL_EXT_device_base");
    if (!platform || !deviceQuery) {
        return nullptr;
    }

    auto getPlatformDisplay = symbol<EGLDisplay (*)(EGLenum, void*, const EGLAttrib*)>(egl, "eglGetPlatformDisplay");
    auto getPlatformDisplayExt = reinterpret_cast<EGLDisplay (*)(EGLenum, void*, const EGLint*)>(getProcAddress("eglGetPlatformDisplayEXT"));
    auto queryDisplayAttrib = reinterpret_cast<EGLBoolean (*)(EGLDisplay, EGLint, EGLAttrib*)>(getProcAddress("eglQueryDisplayAttribEXT"));
    auto queryDeviceString = reinterpret_cast<const char* (*)(EGLDeviceEXT, EGLint)>(getProcAddress("eglQueryDeviceStringEXT"));
    if ((!getPlatformDisplay && !getPlatformDisplayExt) || !queryDisplayAttrib || !queryDeviceString) {
        return nullptr;
    }

    void* display = connect(nullptr);
    if (!display) {
        return nullptr;
    }

    char* node = nullptr;
    EGLDisplay eglDisplay = getPlatformDisplay ? getPlatformDisplay(kEglPlatformWayland, display, nullptr) : getPlatformDisplayExt(kEglPlatformWayland, display, nullptr);
    EGLint major = 0;
    EGLint minor = 0;
    if (eglDisplay && initialize(eglDisplay, &major, &minor)) {
        EGLAttrib device = 0;
        if (queryDisplayAttrib(eglDisplay, kEglDevice, &device) && device) {
            const auto handle = reinterpret_cast<EGLDeviceEXT>(device);
            if (has_extension(queryDeviceString(handle, kEglExtensions), "EGL_EXT_device_drm_render_node")) {
                const char* path = queryDeviceString(handle, kEglDrmRenderNodeFile);
                if (path && *path) {
                    node = qstrdup(path);
                }
            }
        }
        terminate(eglDisplay);
    }
    disconnect(display);
    return node;
#else
    return nullptr;
#endif
}

extern "C" void qstring_init(QString* string, const char* data, size_t len) {
    new (string) QString(QString::fromUtf8(data, static_cast<qsizetype>(len)));
}

extern "C" const ushort* qstring_utf16(const QString* string, size_t* len) {
    *len = static_cast<size_t>(string->size());
    return string->utf16();
}

extern "C" void qstring_drop(QString* string) {
    string->~QString();
}

namespace {

class ApplicationArgs : public QObject {
public:
    ApplicationArgs(const char* const* argv, int argc) {
        for (int i = 0; i < argc; ++i) {
            m_owned.append(QByteArray(argv[i]));
        }
        for (QByteArray& arg : m_owned) {
            m_pointers.emplace_back(arg.data());
            m_size += 1;
        }
    }

    char** data() { return m_pointers.data(); }
    int& size() { return m_size; }

private:
    QList<QByteArray> m_owned;
    int m_size = 0;
    std::vector<char*> m_pointers;
};

}

extern "C" void* application_new(const char* const* argv, int argc) {
    auto* args = new ApplicationArgs(argv, argc);
    auto* app = new QApplication(args->size(), args->data());
    args->setParent(app);
    return app;
}

extern "C" int application_exec() {
    return QApplication::exec();
}

extern "C" void application_delete(void* app) {
    delete static_cast<QApplication*>(app);
}

extern "C" void* qml_engine_load(const char* url, size_t len) {
    auto* engine = new QQmlApplicationEngine();
    engine->load(QUrl(QString::fromUtf8(url, static_cast<qsizetype>(len))));
    return engine;
}

extern "C" void qml_engine_delete(void* engine) {
    delete static_cast<QQmlApplicationEngine*>(engine);
}

extern "C" void init_web_engine() {
    QWebEngineUrlScheme scheme(kFrontendScheme);
    scheme.setSyntax(QWebEngineUrlScheme::Syntax::Host);
    scheme.setFlags(QWebEngineUrlScheme::SecureScheme | QWebEngineUrlScheme::CorsEnabled | QWebEngineUrlScheme::FetchApiAllowed);
    QWebEngineUrlScheme::registerScheme(scheme);

    QtWebEngineQuick::initialize();
}

namespace {

class FrontendScheme : public QWebEngineUrlSchemeHandler {
public:
    FrontendScheme(QString root, QByteArray csp, QList<QByteArray> userScripts, QObject* parent)
        : QWebEngineUrlSchemeHandler(parent), m_root(std::move(root)), m_csp(std::move(csp)), m_userScripts(std::move(userScripts)) {}

    void requestStarted(QWebEngineUrlRequestJob* job) override {
        if (job->requestMethod() != QByteArrayLiteral("GET")) {
            job->fail(QWebEngineUrlRequestJob::RequestDenied);
            return;
        }

        QString path = job->requestUrl().path();
        if (path.isEmpty() || path == QLatin1String("/")) {
            path = QStringLiteral("/index.html");
        }
        if (path.contains(QLatin1String(".."))) {
            job->fail(QWebEngineUrlRequestJob::RequestDenied);
            return;
        }

        QByteArray body;
        if (!shellFile(path, &body)) {
            QFile file(m_root + path);
            if (!file.open(QIODevice::ReadOnly)) {
                job->fail(QWebEngineUrlRequestJob::UrlNotFound);
                return;
            }
            body = file.readAll();
        }

        const QByteArray mime = QMimeDatabase().mimeTypeForFileNameAndData(path, body).name().toUtf8();
        if (mime == QByteArrayLiteral("text/html")) {
            body = withShellScripts(body);
        }

        if (!m_csp.isEmpty()) {
            QMultiMap<QByteArray, QByteArray> headers;
            headers.insert(QByteArrayLiteral("Content-Security-Policy"), m_csp);
            job->setAdditionalResponseHeaders(headers);
        }

        auto* buffer = new QBuffer(job);
        buffer->setData(body);
        buffer->open(QIODevice::ReadOnly);
        job->reply(mime, buffer);
    }

private:
    bool shellFile(const QString& path, QByteArray* body) const {
        if (path == QLatin1String("/__xiao/qwebchannel.js")) {
            QFile client(QStringLiteral(":/qtwebchannel/qwebchannel.js"));
            if (!client.open(QIODevice::ReadOnly)) {
                return false;
            }
            *body = client.readAll();
            return true;
        }
        if (path == QLatin1String("/__xiao/user-scripts.js")) {
            *body = m_userScripts.join(QByteArrayLiteral("\n;\n"));
            return true;
        }
        static const QLatin1String userPrefix("/__xiao/user/");
        if (path.startsWith(userPrefix) && path.endsWith(QLatin1String(".js"))) {
            bool ok = false;
            const int index = path.mid(userPrefix.size(), path.size() - userPrefix.size() - 3).toInt(&ok);
            if (ok && index >= 0 && index < m_userScripts.size()) {
                *body = m_userScripts.at(index);
                return true;
            }
        }
        return false;
    }

    QByteArray withShellScripts(const QByteArray& html) const {
        QByteArray tags = QByteArrayLiteral("<script src=\"/__xiao/qwebchannel.js\"></script>");
        for (qsizetype i = 0; i < m_userScripts.size(); ++i) {
            tags += QByteArrayLiteral("<script src=\"/__xiao/user/") + QByteArray::number(i) + QByteArrayLiteral(".js\"></script>");
        }

        const QByteArray lower = html.toLower();
        qsizetype at = lower.indexOf("<head");
        if (at >= 0) {
            at = lower.indexOf('>', at);
        }
        if (at < 0) {
            return tags + html;
        }
        QByteArray out = html;
        out.insert(at + 1, tags);
        return out;
    }

    QString m_root;
    QByteArray m_csp;
    QList<QByteArray> m_userScripts;
};

class UrlOpenFilter : public QObject {
public:
    UrlOpenFilter(void (*on_url)(const char*), QObject* parent) : QObject(parent), m_on_url(on_url) {}

    bool eventFilter(QObject* watched, QEvent* event) override {
        if (event->type() == QEvent::FileOpen) {
            const QUrl url = static_cast<QFileOpenEvent*>(event)->url();
            if (!url.isLocalFile() && !url.scheme().isEmpty()) {
                m_on_url(url.toString().toUtf8().constData());
                return true;
            }
        }
        return QObject::eventFilter(watched, event);
    }

private:
    void (*m_on_url)(const char*);
};

}

extern "C" void install_frontend_scheme(const char* root, const char* csp, const char* user_scripts_json) {
    const QString root_dir = QString::fromUtf8(root ? root : "");
    const QByteArray policy(csp ? csp : "");

    QList<QByteArray> scripts;
    const QJsonArray sources = QJsonDocument::fromJson(QByteArray(user_scripts_json ? user_scripts_json : "[]")).array();
    for (const QJsonValue& source : sources) {
        scripts.append(source.toString().toUtf8());
    }

    for (QWindow* window : QGuiApplication::topLevelWindows()) {
        for (QObject* child : window->findChildren<QObject*>()) {
            if (!QString::fromLatin1(child->metaObject()->className()).contains(QStringLiteral("WebEngineView"))) {
                continue;
            }
            auto* profile = qobject_cast<QQuickWebEngineProfile*>(child->property("profile").value<QObject*>());
            if (profile && !profile->urlSchemeHandler(kFrontendScheme)) {
                profile->installUrlSchemeHandler(kFrontendScheme, new FrontendScheme(root_dir, policy, scripts, profile));
            }
        }
    }
}

extern "C" void watch_url_open(void (*on_url)(const char*)) {
    if (QCoreApplication* app = QCoreApplication::instance(); app && on_url) {
        app->installEventFilter(new UrlOpenFilter(on_url, app));
    }
}

#ifdef Q_OS_WIN
static QString scheme_key(const char* scheme) { return QStringLiteral("HKEY_CURRENT_USER\\Software\\Classes\\") + QString::fromUtf8(scheme); }

static QString sfx_launcher() {
    const QStringList sfx = qEnvironmentVariable("XIAO_SFX").split(u'|');
    if (sfx.size() == 2 && QFileInfo(sfx[0]) == QFileInfo(QCoreApplication::applicationFilePath())) {
        return QDir::toNativeSeparators(sfx[1]);
    }
    return QString();
}

static QString open_command() {
    QString launcher = sfx_launcher();
    if (launcher.isEmpty()) {
        launcher = QDir::toNativeSeparators(QCoreApplication::applicationFilePath());
    }
    return QStringLiteral("\"%1\" \"%2\"").arg(launcher, QStringLiteral("%1"));
}

static void set_window_property(IPropertyStore* store, const PROPERTYKEY& key, const QString& value) {
    const size_t bytes = (static_cast<size_t>(value.size()) + 1) * sizeof(wchar_t);
    PROPVARIANT variant;
    PropVariantInit(&variant);
    variant.vt = VT_LPWSTR;
    variant.pwszVal = static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if (!variant.pwszVal) {
        return;
    }
    memcpy(variant.pwszVal, value.utf16(), bytes);
    store->SetValue(key, variant);
    PropVariantClear(&variant);
}
#endif

extern "C" void set_relaunch_command(const char* app_id, const char* display_name) {
#ifdef Q_OS_WIN
    const QString launcher = sfx_launcher();
    if (launcher.isEmpty() || !app_id || !*app_id || !display_name) {
        return;
    }

    constexpr GUID app_model = {0x9F4C2855, 0x9F79, 0x4B39, {0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3}};
    const std::pair<PROPERTYKEY, QString> values[] = {
        {{app_model, 5}, QString::fromUtf8(app_id)},
        {{app_model, 2}, QStringLiteral("\"%1\"").arg(launcher)},
        {{app_model, 4}, QString::fromUtf8(display_name)},
        {{app_model, 3}, launcher + QStringLiteral(",0")},
    };

    for (QWindow* window : QGuiApplication::topLevelWindows()) {
        if (!qobject_cast<QQuickWindow*>(window)) {
            continue;
        }
        IPropertyStore* store = nullptr;
        if (FAILED(SHGetPropertyStoreForWindow(reinterpret_cast<HWND>(window->winId()), IID_PPV_ARGS(&store)))) {
            continue;
        }
        for (const auto& [key, value] : values) {
            set_window_property(store, key, value);
        }
        store->Commit();
        store->Release();
    }
#else
    Q_UNUSED(app_id);
    Q_UNUSED(display_name);
#endif
}

extern "C" bool register_url_scheme(const char* scheme) {
#ifdef Q_OS_WIN
    if (!scheme || !*scheme || !QCoreApplication::instance()) {
        return false;
    }
    QSettings key(scheme_key(scheme), QSettings::NativeFormat);
    key.setValue(QStringLiteral("Default"), QStringLiteral("URL:%1").arg(QString::fromUtf8(scheme)));
    key.setValue(QStringLiteral("URL Protocol"), QString());
    key.setValue(QStringLiteral("shell/open/command/Default"), open_command());
    key.sync();
    return key.status() == QSettings::NoError;
#else
    Q_UNUSED(scheme);
    return false;
#endif
}

extern "C" bool url_scheme_registered(const char* scheme) {
#ifdef Q_OS_WIN
    if (!scheme || !*scheme || !QCoreApplication::instance()) {
        return false;
    }
    QSettings key(scheme_key(scheme), QSettings::NativeFormat);
    return key.value(QStringLiteral("shell/open/command/Default")).toString() == open_command();
#else
    Q_UNUSED(scheme);
    return false;
#endif
}

extern "C" bool unregister_url_scheme(const char* scheme) {
#ifdef Q_OS_WIN
    if (!scheme || !*scheme) {
        return false;
    }
    QSettings key(scheme_key(scheme), QSettings::NativeFormat);
    key.remove(QString());
    key.sync();
    return key.status() == QSettings::NoError;
#else
    Q_UNUSED(scheme);
    return false;
#endif
}

extern "C" char* standard_path(int location, const char* scope) {
    QString path = QStandardPaths::writableLocation(static_cast<QStandardPaths::StandardLocation>(location));
    if (path.isEmpty()) {
        return nullptr;
    }

    if (scope) {
        QString suffix;
        if (const QString org = QCoreApplication::organizationName(); !org.isEmpty()) {
            suffix += QLatin1Char('/') + org;
        }
        if (const QString app = QCoreApplication::applicationName(); !app.isEmpty()) {
            suffix += QLatin1Char('/') + app;
        }

        const QString replacement = *scope ? QLatin1Char('/') + QString::fromUtf8(scope) : QString();
        const qsizetype at = suffix.isEmpty() ? -1 : path.lastIndexOf(suffix);
        if (at >= 0) {
            path.replace(at, suffix.size(), replacement);
        } else {
            path += replacement;
        }
    }
    return qstrdup(QDir::toNativeSeparators(path).toUtf8().constData());
}

extern "C" char* application_dir() {
    if (!QCoreApplication::instance()) {
        return nullptr;
    }
    return qstrdup(QDir::toNativeSeparators(QCoreApplication::applicationDirPath()).toUtf8().constData());
}

extern "C" char* timestamp(bool local) {
    const QDateTime now = local ? QDateTime::currentDateTime() : QDateTime::currentDateTimeUtc();
    return qstrdup(now.toString(QStringLiteral("yyyy-MM-dd HH:mm:ss.zzz")).toUtf8().constData());
}

extern "C" bool set_window_icon_data(const unsigned char* data, size_t size) {
    QPixmap pixmap;
    if (!data || !size || !pixmap.loadFromData(data, static_cast<uint>(size))) {
        return false;
    }
    QGuiApplication::setWindowIcon(QIcon(pixmap));
    return true;
}

extern "C" char* read_resource(const char* path) {
    QFile file(QString::fromUtf8(path ? path : ""));
    if (!file.open(QIODevice::ReadOnly)) {
        return nullptr;
    }
    return qstrdup(file.readAll().constData());
}

namespace {

class RepaintGuard : public QObject {
public:
    explicit RepaintGuard(QQuickWindow* window) : QObject(window), m_window(window) {
        connect(window, &QWindow::visibilityChanged, this, [this](QWindow::Visibility visibility) {
            if (visibility == QWindow::Hidden || visibility == QWindow::Minimized) {
                showViews(false);
            } else {
                wake();
            }
        });

        connect(window, &QQuickWindow::sceneGraphInvalidated, this, [this] { m_lost = true; }, Qt::QueuedConnection);
        connect(window, &QQuickWindow::sceneGraphInitialized, this,
            [this] {
                if (m_lost) {
                    m_lost = false;
                    showViews(false);
                    wake();
                }
            },
            Qt::QueuedConnection);

        m_awayTimer.setSingleShot(true);
        m_awayTimer.setInterval(150);
        connect(&m_awayTimer, &QTimer::timeout, this, [this] {
            if (!m_window->isExposed()) {
                m_unexposed = true;
                showViews(false);
            }
        });
        window->installEventFilter(this);
    }

protected:
    bool eventFilter(QObject* watched, QEvent* event) override {
        if (watched == m_window && event->type() == QEvent::Expose) {
            if (!m_window->isExposed()) {
                if (!m_unexposed && !m_awayTimer.isActive()) {
                    m_awayTimer.start();
                }
            } else {
                m_awayTimer.stop();
                if (m_unexposed) {
                    m_unexposed = false;
                    wake();
                }
            }
        }
        return QObject::eventFilter(watched, event);
    }

private:
    QList<QPointer<QObject>> views() const {
        QList<QPointer<QObject>> found;
        for (QObject* child : m_window->findChildren<QObject*>()) {
            if (QString::fromLatin1(child->metaObject()->className()).contains(QStringLiteral("WebEngineView"))) {
                found.append(child);
            }
        }
        return found;
    }

    void showViews(bool visible) {
        for (const QPointer<QObject>& view : views()) {
            if (view) {
                view->setProperty("visible", visible);
            }
        }
    }

    void wake() {
        if (m_waking) {
            return;
        }
        m_waking = true;

        QTimer::singleShot(32, this, [this] {
            const QList<QPointer<QObject>> awake = views();
            for (const QPointer<QObject>& view : awake) {
                if (view) {
                    view->setProperty("visible", true);
                    view->setProperty("opacity", 0.99);
                }
            }
            QTimer::singleShot(32, this, [this, awake] {
                for (const QPointer<QObject>& view : awake) {
                    if (view) {
                        view->setProperty("opacity", 1.0);
                    }
                }
                m_waking = false;
            });
        });
    }

    QQuickWindow* m_window;
    QTimer m_awayTimer;
    bool m_lost = false;
    bool m_waking = false;
    bool m_unexposed = false;
};

}

extern "C" void install_repaint_guard() {
    const QList<QWindow*> windows = QGuiApplication::topLevelWindows();
    for (QWindow* window : windows) {
        if (QQuickWindow* quick = qobject_cast<QQuickWindow*>(window)) {
            new RepaintGuard(quick);
        }
    }
}

extern "C" bool single_instance_listen(const char* name, void (*on_message)(const char*)) {
    if (!name || !*name || !on_message) {
        return false;
    }

    const QString server_name = QString::fromUtf8(name);
    QLocalServer::removeServer(server_name);

    auto* server = new QLocalServer(QCoreApplication::instance());
    server->setSocketOptions(QLocalServer::UserAccessOption);
    if (!server->listen(server_name)) {
        delete server;
        return false;
    }

    QObject::connect(server, &QLocalServer::newConnection, server, [server, on_message] {
        while (QLocalSocket* socket = server->nextPendingConnection()) {
            auto drain = [socket, on_message] {
                while (socket->canReadLine()) {
                    const QByteArray line = socket->readLine().trimmed();
                    if (!line.isEmpty()) {
                        on_message(line.constData());
                    }
                }
            };
            QObject::connect(socket, &QLocalSocket::readyRead, socket, drain);
            QObject::connect(socket, &QLocalSocket::disconnected, socket, [socket, drain] {
                drain();
                socket->deleteLater();
            });
            drain();
        }
    });
    return true;
}

extern "C" void set_app_identity(const char* name, const char* organization) {
    if (name && *name) {
        QCoreApplication::setApplicationName(QString::fromUtf8(name));
        QGuiApplication::setDesktopFileName(QString::fromUtf8(name));
    }
    if (organization && *organization) {
        QCoreApplication::setOrganizationName(QString::fromUtf8(organization));
    }
}

extern "C" bool resource_exists(const char* path) {
    return path && *path && QFile::exists(QString::fromUtf8(path));
}

extern "C" bool open_url(const char* url) {
    if (!url || !*url) {
        return false;
    }
    const QUrl parsed(QString::fromUtf8(url), QUrl::TolerantMode);
    return parsed.isValid() && !parsed.scheme().isEmpty() && QDesktopServices::openUrl(parsed);
}

extern "C" bool open_path(const char* path) {
    if (!path || !*path) {
        return false;
    }
    return QDesktopServices::openUrl(QUrl::fromLocalFile(QString::fromUtf8(path)));
}

extern "C" bool reveal_path(const char* path) {
    if (!path || !*path) {
        return false;
    }
    const QString target = QString::fromUtf8(path);

#if defined(Q_OS_LINUX)
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        return false;
    }

    QDBusInterface manager(QStringLiteral("org.freedesktop.FileManager1"), QStringLiteral("/org/freedesktop/FileManager1"), QStringLiteral("org.freedesktop.FileManager1"), bus);
    if (!manager.isValid()) {
        return false;
    }

    const QDBusMessage reply = manager.call(QStringLiteral("ShowItems"), QStringList{QUrl::fromLocalFile(target).toString()}, QString());
    return reply.type() != QDBusMessage::ErrorMessage;
#elif defined(Q_OS_WIN)
    const QString native = QDir::toNativeSeparators(target);
    PIDLIST_ABSOLUTE item = ILCreateFromPathW(reinterpret_cast<const wchar_t*>(native.utf16()));
    if (!item) {
        return false;
    }

    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    const HRESULT opened = SHOpenFolderAndSelectItems(item, 0, nullptr, 0);
    ILFree(item);
    return SUCCEEDED(opened);
#elif defined(Q_OS_MACOS)
    return reveal_path_mac(target.toUtf8().constData());
#else
    return false;
#endif
}

static QClipboard* board() {
    QGuiApplication* app = qobject_cast<QGuiApplication*>(QCoreApplication::instance());
    if (!app || QThread::currentThread() != app->thread()) {
        return nullptr;
    }
    return QGuiApplication::clipboard();
}

extern "C" void string_free(char* text) {
    delete[] text;
}

extern "C" int clipboard_kinds() {
    QClipboard* clipboard = board();
    if (!clipboard) {
        return -1;
    }

    const QMimeData* mime = clipboard->mimeData();
    if (!mime) {
        return 0;
    }

    int kinds = 0;
    if (mime->hasText()) {
        kinds |= 1;
    }
    if (mime->hasHtml()) {
        kinds |= 2;
    }
    if (mime->hasImage()) {
        kinds |= 4;
    }
    return kinds;
}

extern "C" char* clipboard_read_text() {
    QClipboard* clipboard = board();
    return clipboard ? qstrdup(clipboard->text().toUtf8().constData()) : nullptr;
}

extern "C" bool clipboard_write_text(const char* text) {
    QClipboard* clipboard = board();
    if (!clipboard || !text) {
        return false;
    }
    clipboard->setText(QString::fromUtf8(text));
    return true;
}

extern "C" char* clipboard_read_html() {
    QClipboard* clipboard = board();
    if (!clipboard) {
        return nullptr;
    }

    const QMimeData* mime = clipboard->mimeData();
    return mime && mime->hasHtml() ? qstrdup(mime->html().toUtf8().constData()) : nullptr;
}

extern "C" bool clipboard_write_html(const char* html, const char* plain) {
    QClipboard* clipboard = board();
    if (!clipboard || !html) {
        return false;
    }

    QMimeData* mime = new QMimeData;
    mime->setHtml(QString::fromUtf8(html));
    mime->setText(QString::fromUtf8(plain && *plain ? plain : html));
    clipboard->setMimeData(mime);
    return true;
}

extern "C" char* clipboard_read_image() {
    QClipboard* clipboard = board();
    if (!clipboard) {
        return nullptr;
    }

    const QImage image = clipboard->image();
    if (image.isNull()) {
        return nullptr;
    }

    QByteArray png;
    QBuffer buffer(&png);
    if (!buffer.open(QIODevice::WriteOnly) || !image.save(&buffer, "PNG")) {
        return nullptr;
    }
    return qstrdup(png.toBase64().constData());
}

extern "C" bool clipboard_write_image(const char* base64) {
    QClipboard* clipboard = board();
    if (!clipboard || !base64) {
        return false;
    }

    QByteArray payload(base64);
    const qsizetype marker = payload.indexOf("base64,");
    if (marker >= 0) {
        payload = payload.mid(marker + 7);
    }

    QImage image;
    if (!image.loadFromData(QByteArray::fromBase64(payload))) {
        return false;
    }
    clipboard->setImage(image);
    return true;
}

extern "C" bool clipboard_write_image_file(const char* path) {
    QClipboard* clipboard = board();
    if (!clipboard || !path || !*path) {
        return false;
    }

    const QImage image(QString::fromUtf8(path));
    if (image.isNull()) {
        return false;
    }
    clipboard->setImage(image);
    return true;
}

extern "C" bool clipboard_clear() {
    QClipboard* clipboard = board();
    if (!clipboard) {
        return false;
    }
    clipboard->clear();
    return true;
}

extern "C" void set_window_icon(const char* path) {
    if (path && *path) {
        QGuiApplication::setWindowIcon(QIcon(QString::fromUtf8(path)));
    }
}

extern "C" bool notify(const char* app, const char* title, const char* body) {
#ifdef Q_OS_LINUX
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        return false;
    }

    QDBusInterface notifications(QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("/org/freedesktop/Notifications"), QStringLiteral("org.freedesktop.Notifications"), bus);
    if (!notifications.isValid()) {
        return false;
    }

    QVariantMap hints;
    hints.insert(QStringLiteral("desktop-entry"), QString::fromUtf8(app));

    const QDBusReply<uint> reply = notifications.call(QStringLiteral("Notify"), QString::fromUtf8(app), 0u, QString(), QString::fromUtf8(title),
                                                      QString::fromUtf8(body), QStringList(), hints, -1);
    return reply.isValid();
#else
    Q_UNUSED(app);
    Q_UNUSED(title);
    Q_UNUSED(body);
    return false;
#endif
}
