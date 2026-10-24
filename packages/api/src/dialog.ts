import { invoke, listen } from "./core";

export interface OpenOptions {
    title?: string;
    filters?: string[];
    defaultPath?: string;
}

export interface MessageOptions {
    title?: string;
    text: string;
    detail?: string;
    buttons?: "ok" | "okcancel" | "yesno";
}

interface DialogResult {
    cancelled: boolean;
    paths?: string[];
    button?: "ok" | "cancel" | "yes" | "no";
    error?: string;
}

let nextId = 0;

async function request(kind: string, args: Record<string, unknown>): Promise<DialogResult> {
    const id = `j${Date.now().toString(36)}-${(nextId++).toString(36)}`;
    let settle!: (value: DialogResult) => void;
    const answered = new Promise<DialogResult>((resolve) => (settle = resolve));

    const unlisten = await listen<DialogResult>(`dialog://result/${id}`, settle);
    try {
        await invoke<string>("xiao:dialog", { ...args, kind, id });
    } catch (err) {
        unlisten();
        throw err;
    }
    const result = await answered;
    unlisten();

    if (result.error) throw new Error(result.error);
    return result;
}

/** Native dialogs. Only one is shown at a time; later requests wait their turn. Cancelling gives `[]` or `null`. */
export const dialog = {
    async open(options: OpenOptions = {}): Promise<string[]> {
        const r = await request("open", options as Record<string, unknown>);
        return r.cancelled ? [] : (r.paths ?? []);
    },

    async folder(options: OpenOptions = {}): Promise<string | null> {
        const r = await request("folder", options as Record<string, unknown>);
        return r.cancelled ? null : (r.paths?.[0] ?? null);
    },

    async save(options: OpenOptions = {}): Promise<string | null> {
        const r = await request("save", options as Record<string, unknown>);
        return r.cancelled ? null : (r.paths?.[0] ?? null);
    },

    async message(options: MessageOptions): Promise<"ok" | "cancel" | "yes" | "no"> {
        const r = await request("message", options as unknown as Record<string, unknown>);
        return r.button ?? "cancel";
    },

    async confirm(text: string, title?: string): Promise<boolean> {
        return (await dialog.message({ text, title, buttons: "yesno" })) === "yes";
    },
};
