import { invoke } from "./core";

/**
 * `open` hands the path to the application configured for its type — the text
 * editor for a text file, the file manager for a folder. `reveal` opens the
 * file manager with the item selected, falling back to its containing folder
 * where no file manager answers.
 */
export type OpenMode = "open" | "reveal";

export const opener = {
    /** Opens a URL in its default handler. Allowed: http, https, mailto, the app's deep-link schemes and `app.security.opener.schemes`. */
    async openUrl(url: string): Promise<void> {
        await invoke<null>("xiao:opener", { url });
    },

    /** Opens or reveals a path inside `app.security.opener.paths`. Opening an executable is refused. */
    async openPath(path: string, mode: OpenMode = "open"): Promise<void> {
        await invoke<null>("xiao:opener", { path, mode });
    },
};
