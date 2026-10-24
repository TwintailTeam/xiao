type Envelope<T> = { ok: true; data: T } | { ok: false; error: string } | { ok: true; pending: string };
export type UnlistenFn = () => void;

export interface XiaoBackend {
    invoke(cmd: string, args: string, callback: (result: string) => void): void;
    emitEvent(name: string, payload: string): void;
    frontendReady(): void;
    startUrl(callback: (url: string) => void): void;
    isDev(callback: (dev: boolean) => void): void;
    config(callback: (json: string) => void): void;
    rustEvent: {
        connect(handler: (name: string, payload: string) => void): void;
    };
    invokeResult: {
        connect(handler: (id: string, envelope: string) => void): void;
    };
}

export interface XiaoChannel {
    objects: Record<string, XiaoBackend & Record<string, unknown>>;
}

declare global {
    interface Window {
        qt?: { webChannelTransport: unknown };
        QWebChannel: new (transport: unknown, callback: (channel: XiaoChannel) => void) => void;
    }
}

interface XiaoState {
    connection: Promise<XiaoBackend> | null;
    channel: XiaoChannel | null;
    listeners: Map<string, Set<(payload: any) => void>>;
    rawListeners: Set<(name: string, payload: string) => void>;
    pending: Map<string, (envelope: string) => void>;
}

const globals = globalThis as unknown as { __xiao_state__?: XiaoState };
const state: XiaoState = (globals.__xiao_state__ ??= {
    connection: null,
    channel: null,
    listeners: new Map(),
    rawListeners: new Set(),
    pending: new Map(),
});

const listeners = state.listeners;

/** Whether the page runs inside a xiao app rather than a browser tab. */
export function isXiaoShell(): boolean {
    return typeof window !== "undefined" && !!window.qt?.webChannelTransport;
}

const SHELL = "xiao://localhost/__xiao";

function loadScript(src: string): Promise<void> {
    return new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = src;
        script.onload = () => resolve();
        script.onerror = () => reject(new Error(`qwebchannel.js was not injected and ${src} did not load: check the Content-Security-Policy's script-src`));
        (document.head || document.documentElement).appendChild(script);
    });
}

async function loadChannelClient(): Promise<void> {
    if (typeof window.QWebChannel === "function") return;
    await loadScript(`${SHELL}/qwebchannel.js`);
    await loadScript(`${SHELL}/user-scripts.js`);
}

/** Connects to Rust over the web channel. Every other call does this first; the connection is made once and shared. */
export function connect(): Promise<XiaoBackend> {
    if (state.connection) return state.connection;

    const client = isXiaoShell() ? loadChannelClient() : Promise.resolve();
    state.connection = client.then(() => new Promise<XiaoBackend>((resolve, reject) => {
        if (!isXiaoShell()) {
            reject(new Error("not running inside the Qt shell: open the app with `npm run dev`, not in a browser tab"));
            return;
        }

        new window.QWebChannel(window.qt!.webChannelTransport, (channel) => {
            const backend = channel.objects.backend;
            if (!backend) {
                reject(new Error("no `backend` object on the web channel"));
                return;
            }
            state.channel = channel;

            backend.invokeResult.connect((id: string, envelope: string) => {
                const settle = state.pending.get(id);
                state.pending.delete(id);
                settle?.(envelope);
            });

            backend.rustEvent.connect((name: string, payload: string) => {
                state.rawListeners.forEach((handler) => {
                    try {
                        handler(name, payload);
                    } catch (err) {
                        console.error("raw channel listener threw:", err);
                    }
                });

                const handlers = listeners.get(name);
                if (!handlers || handlers.size === 0) return;

                let parsed: unknown = null;
                try {
                    parsed = payload ? JSON.parse(payload) : null;
                } catch {
                    parsed = payload;
                }

                handlers.forEach((handler) => {
                    try {
                        handler(parsed);
                    } catch (err) {
                        console.error(`listener for "${name}" threw:`, err);
                    }
                });
            });

            console.info("[xiao] connected to Rust backend over QWebChannel");
            resolve(backend);
            setTimeout(() => backend.frontendReady(), 0);
        });
    }));

    return state.connection;
}

/** Calls a Rust command by name and resolves with what it returned. */
export async function invoke<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const backend = await connect();

    const parse = (raw: string): Envelope<T> => {
        try {
            return JSON.parse(raw) as Envelope<T>;
        } catch {
            throw new Error(`command \`${cmd}\` returned malformed JSON: ${raw}`);
        }
    };

    let envelope = parse(
        await new Promise<string>((resolve) => {
            backend.invoke(cmd, JSON.stringify(args), resolve);
        }),
    );

    if (envelope.ok && "pending" in envelope) {
        const id = envelope.pending;
        envelope = parse(await new Promise<string>((resolve) => state.pending.set(id, resolve)));
    }

    if (!envelope.ok) throw new Error(envelope.error);
    if ("pending" in envelope) throw new Error(`command \`${cmd}\` answered twice with a ticket`);
    return envelope.data;
}

/** Subscribes to an event Rust emits. */
export async function listen<T = unknown>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
    await connect();

    if (!listeners.has(event)) listeners.set(event, new Set());
    const bucket = listeners.get(event)!;
    bucket.add(handler as (payload: any) => void);

    return () => {
        bucket.delete(handler as (payload: any) => void);
    };
}

/** Subscribes to an event, then unsubscribes as soon as it has fired once. */
export async function once<T = unknown>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
    const unlisten = await listen<T>(event, (payload) => {
        unlisten();
        handler(payload);
    });
    return unlisten;
}

/** Emits an event to Rust listeners and to the page's own. */
export async function emit(event: string, payload: unknown = {}): Promise<void> {
    const backend = await connect();
    backend.emitEvent(event, JSON.stringify(payload));
}

/** Tells Rust the page is up, which flushes any events buffered before it was. */
export async function ready(): Promise<void> {
    const backend = await connect();
    backend.frontendReady();
}

/** The app's `xiao.conf.json`. */
export async function config<T = Record<string, unknown>>(): Promise<T> {
    const backend = await connect();
    const json = await new Promise<string>((resolve) => backend.config(resolve));
    return JSON.parse(json) as T;
}

export function channelState(): XiaoState {
    return state;
}
