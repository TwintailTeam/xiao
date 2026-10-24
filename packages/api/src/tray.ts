import { invoke, listen, type UnlistenFn } from "./core";

export interface TrayMenuItem {
    id: string;
    text: string;
    enabled?: boolean;
    separator?: boolean;
    checked?: boolean;
}

export enum TrayActivation {
    Unknown = 0,
    Context = 1,
    DoubleClick = 2,
    Trigger = 3,
    MiddleClick = 4,
}

const call = (action: string, args: Record<string, unknown> = {}) => invoke<null>("xiao:tray", { action, ...args });

/** The tray icon from `app.trayIcon`. */
export const tray = {
    show: () => call("show"),
    hide: () => call("hide"),
    setTooltip: (value: string) => call("setTooltip", { value }),
    setIcon: (value: string) => call("setIcon", { value }),
    setMenu: (items: TrayMenuItem[]) => call("setMenu", { items }),
    /** Opens the menu on left-click too. On by default; on Wayland the status bar decides. */
    setMenuOnLeftClick: (value: boolean) => call("setMenuOnLeftClick", { value }),
    notify: (title: string, message: string) => call("notify", { title, message }),
    onMenuClick: (handler: (item: { id: string; checked: boolean }) => void): Promise<UnlistenFn> => listen("tray://menu", handler),
    onActivated: (handler: (reason: TrayActivation) => void): Promise<UnlistenFn> => listen<{ reason: number }>("tray://activated", (p) => handler(p.reason as TrayActivation)),
    onMessageClicked: (handler: () => void): Promise<UnlistenFn> => listen("tray://message-clicked", handler),
};
