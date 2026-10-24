import { channelState, connect, invoke, isXiaoShell, type UnlistenFn, type XiaoBackend, type XiaoChannel } from "./core";

export interface WebChannelInfo {
    attached: boolean;
    csp: string | null;
    storage: {
        storageName: string;
        storagePath: string;
        cachePath: string;
        downloadPath: string;
    };
}

const call = <T>(action: string, args: Record<string, unknown> = {}) => invoke<T>("xiao:webchannel", { action, ...args });

/** The QWebChannel that `invoke`, `emit` and `listen` run on, for objects registered from QML or unparsed events. */
export const webchannel = {
    /** Whether a Qt transport is present at all. False in a plain browser tab. */
    isAvailable: isXiaoShell,
    /** The raw Qt transport object, `window.qt.webChannelTransport`. */
    transport: (): unknown => (typeof window === "undefined" ? undefined : window.qt?.webChannelTransport),
    /** The `Backend` QObject, with every signal and invokable Rust exposed. */
    backend: (): Promise<XiaoBackend> => connect(),
    /** The whole channel, once connected. */
    async channel(): Promise<XiaoChannel> {
        await connect();
        return channelState().channel!;
    },

    /** Every object on the channel, including any the app registered from QML. */
    async objects(): Promise<XiaoChannel["objects"]> {
        return (await webchannel.channel()).objects;
    },

    /** One object by its `WebChannel.id`. */
    async object<T = Record<string, unknown>>(id: string): Promise<T | undefined> {
        return (await webchannel.objects())[id] as T | undefined;
    },

    /** Subscribes to every event, with the payload as the unparsed string Rust sent. */
    async onRaw(handler: (name: string, payload: string) => void): Promise<UnlistenFn> {
        await connect();
        const { rawListeners } = channelState();
        rawListeners.add(handler);
        return () => {
            rawListeners.delete(handler);
        };
    },

    /** Pushes a message onto the transport without going through `invoke`. */
    async send(message: unknown): Promise<void> {
        await connect();
        const transport = webchannel.transport() as { send?: (data: string) => void } | undefined;
        if (!transport?.send) throw new Error("the Qt transport does not expose send()");
        transport.send(JSON.stringify(message));
    },

    /** Whether Rust considers the page attached, the policy in force, and the storage paths. */
    info: (): Promise<WebChannelInfo> => call<WebChannelInfo>("info"),
};
