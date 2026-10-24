import { invoke, listen, type UnlistenFn } from "./core";

const call = <T>(action: string, args: Record<string, unknown> = {}) => invoke<T>("xiao:deep-link", { action, ...args });

/**
 * OS-level URL schemes that open the app — `myapp://…` clicked anywhere on the
 * desktop. List them in `app.deepLink.schemes`, or register more at runtime on
 * Linux and Windows.
 */
export const deepLink = {
    /** The links the app was most recently opened with. */
    getCurrent: (): Promise<string[]> => call<string[]>("current"),
    /** Fires for every link that arrives while the app runs. */
    onOpenUrl: (handler: (urls: string[]) => void): Promise<UnlistenFn> => listen<{ urls: string[] }>("deep-link://new-url", (p) => handler(p.urls)),
    /** Makes the app the current user's handler for `scheme`. Rejects on macOS, where only Info.plist counts. */
    register: (scheme: string): Promise<null> => call<null>("register", { scheme }),
    unregister: (scheme: string): Promise<null> => call<null>("unregister", { scheme }),
    isRegistered: (scheme: string): Promise<boolean> => call<boolean>("isRegistered", { scheme }),
};
