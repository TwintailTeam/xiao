import { invoke } from "./core";

export type PathName =
    | "home" | "config" | "data" | "localData" | "cache" | "runtime" | "temp"
    | "desktop" | "document" | "download" | "picture" | "public" | "audio"
    | "video" | "font" | "executable" | "appConfig" | "appData"
    | "appLocalData" | "appCache" | "appLog" | "resource";

export interface PathOptions {
    /** A relative path to append. It may not climb out with `..` or be absolute. */
    join?: string;
    /** Creates the directory. Needs `app.security.page.createDirectories`. */
    create?: boolean;
}

const dir = (name: PathName) => (opts: PathOptions = {}) =>
    invoke<string>("xiao:path", { name, ...opts });

/** The platform's directories. The `app*` ones are scoped to the app's `identifier`. Rejects for a name the platform has no directory for. */
export const path = {
    resolve: (name: PathName, opts: PathOptions = {}) => invoke<string>("xiao:path", { name, ...opts }),
    all: () => invoke<Record<PathName, string | null>>("xiao:path", {}),
    homeDir: dir("home"),
    configDir: dir("config"),
    dataDir: dir("data"),
    localDataDir: dir("localData"),
    cacheDir: dir("cache"),
    runtimeDir: dir("runtime"),
    tempDir: dir("temp"),
    desktopDir: dir("desktop"),
    documentDir: dir("document"),
    downloadDir: dir("download"),
    pictureDir: dir("picture"),
    publicDir: dir("public"),
    audioDir: dir("audio"),
    videoDir: dir("video"),
    fontDir: dir("font"),
    executableDir: dir("executable"),
    appConfigDir: dir("appConfig"),
    appDataDir: dir("appData"),
    appLocalDataDir: dir("appLocalData"),
    appCacheDir: dir("appCache"),
    appLogDir: dir("appLog"),
    resourceDir: dir("resource"),
};
