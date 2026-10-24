import { invoke, listen, type UnlistenFn } from "./core";

type Level = "trace" | "debug" | "info" | "warn" | "error";

const write = (level: Level) => (message: string) => invoke<null>("xiao:log", { level, message });

/**
 * The app's `log` logger, from the page — the same file and targets Rust logs
 * to, under the target `webview`. `attachConsole` prints what Rust logs to the
 * Webview target into this page's console, as in `tauri-plugin-log`.
 */
export const log = {
    trace: write("trace"),
    debug: write("debug"),
    info: write("info"),
    warn: write("warn"),
    error: write("error"),
    attachConsole: (): Promise<UnlistenFn> =>
        listen<{ level: string; target: string; message: string }>("log://log", (p) => {
            const line = `[${p.target}] ${p.message}`;
            switch (p.level) {
                case "ERROR":
                    console.error(line);
                    break;
                case "WARN":
                    console.warn(line);
                    break;
                case "INFO":
                    console.info(line);
                    break;
                default:
                    console.debug(line);
            }
        }),
};
