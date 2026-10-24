import { invoke } from "./core";

/** Which formats the clipboard is holding right now. */
export type ClipboardKinds = { text: boolean; html: boolean; image: boolean };

/** The system clipboard. Reads need `app.security.page.readClipboard`, writes `writeClipboard`. */
export const clipboard = {
    async writeText(text: string): Promise<void> {
        await invoke<null>("xiao:clipboard", { action: "writeText", text });
    },

    async readText(): Promise<string> {
        return invoke<string>("xiao:clipboard", { action: "readText" });
    },

    /** `text` is what applications that take no markup will paste; the html is used when it is left out. */
    async writeHtml(html: string, text?: string): Promise<void> {
        await invoke<null>("xiao:clipboard", { action: "writeHtml", html, text });
    },

    async readHtml(): Promise<string> {
        return invoke<string>("xiao:clipboard", { action: "readHtml" });
    },

    /** Takes base64 image bytes, with or without the `data:image/png;base64,` prefix a canvas gives you. */
    async writeImage(image: string): Promise<void> {
        await invoke<null>("xiao:clipboard", { action: "writeImage", image });
    },

    /** Base64 PNG, no `data:` prefix — add one to put it in an `<img>`. */
    async readImage(): Promise<string> {
        return invoke<string>("xiao:clipboard", { action: "readImage" });
    },

    async clear(): Promise<void> {
        await invoke<null>("xiao:clipboard", { action: "clear" });
    },

    async kinds(): Promise<ClipboardKinds> {
        return invoke<ClipboardKinds>("xiao:clipboard", { action: "kinds" });
    },
};
