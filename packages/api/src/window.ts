import { invoke, listen, type UnlistenFn } from "./core";

const win = (action: string, args: Record<string, unknown> = {}) => invoke<null>("xiao:window", { action, ...args });

export interface WindowSize {
    width: number;
    height: number;
}

export interface WindowPosition {
    x: number;
    y: number;
}

export type WindowTheme = "light" | "dark" | "unknown";

export interface CloseRequest {
    /** False when the window is going away; true when `setPreventClose` held it. */
    prevented: boolean;
}

export const WindowVisibility = {
    Hidden: 0,
    AutomaticVisibility: 1,
    Windowed: 2,
    Minimized: 3,
    Maximized: 4,
    FullScreen: 5,
} as const;

export type WindowVisibilityValue = (typeof WindowVisibility)[keyof typeof WindowVisibility];

/** The app's window. */
export const appWindow = {
    show: () => win("show"),
    hide: () => win("hide"),
    close: () => win("close"),
    destroy: () => win("destroy"),
    minimize: () => win("minimize"),
    maximize: () => win("maximize"),
    unmaximize: () => win("unmaximize"),
    toggleMaximize: () => win("toggleMaximize"),
    center: () => win("center"),
    focus: () => win("focus"),
    setFullscreen: (value: boolean) => win("setFullscreen", { value }),
    setResizable: (value: boolean) => win("setResizable", { value }),
    setAlwaysOnTop: (value: boolean) => win("setAlwaysOnTop", { value }),
    setPreventClose: (value: boolean) => win("setPreventClose", { value }),
    setTitle: (value: string) => win("setTitle", { value }),
    setSize: (width: number, height: number) => win("setSize", { width, height }),
    setMinSize: (width: number, height: number) => win("setMinSize", { width, height }),
    setPosition: (x: number, y: number) => win("setPosition", { x, y }),
    onCloseRequested: (handler: (event: CloseRequest) => void): Promise<UnlistenFn> => listen<CloseRequest>("window://close-requested", handler),
    onDestroyed: (handler: () => void): Promise<UnlistenFn> => listen("window://destroyed", handler),
    onResized: (handler: (size: WindowSize) => void): Promise<UnlistenFn> => listen<WindowSize>("window://resized", handler),
    onMoved: (handler: (position: WindowPosition) => void): Promise<UnlistenFn> => listen<WindowPosition>("window://moved", handler),
    onScaleChanged: (handler: (scaleFactor: number) => void): Promise<UnlistenFn> =>
        listen<{ scaleFactor: number }>("window://scale-changed", (p) => handler(p.scaleFactor)),
    onThemeChanged: (handler: (theme: WindowTheme) => void): Promise<UnlistenFn> =>
        listen<{ theme: WindowTheme }>("window://theme-changed", (p) => handler(p.theme)),
    onFocusChanged: async (handler: (focused: boolean) => void): Promise<UnlistenFn> => {
        const uns = await Promise.all([listen("window://focus", () => handler(true)), listen("window://blur", () => handler(false))]);
        return () => uns.forEach((un) => un());
    },
    onVisibilityChanged: (handler: (visibility: WindowVisibilityValue) => void): Promise<UnlistenFn> =>
        listen<{ visibility: WindowVisibilityValue }>("window://visibility", (p) => handler(p.visibility)),
    onMinimized: (handler: () => void): Promise<UnlistenFn> =>
        appWindow.onVisibilityChanged((v) => {
            if (v === WindowVisibility.Minimized) handler();
        }),
    onMaximized: (handler: () => void): Promise<UnlistenFn> =>
        appWindow.onVisibilityChanged((v) => {
            if (v === WindowVisibility.Maximized) handler();
        }),
    onRestored: (handler: (visibility: WindowVisibilityValue) => void): Promise<UnlistenFn> => {
        let previous: number = -1;
        return appWindow.onVisibilityChanged((v) => {
            const was = previous;
            previous = v;
            if (was === WindowVisibility.Minimized && v !== WindowVisibility.Minimized) handler(v);
        });
    },
};
