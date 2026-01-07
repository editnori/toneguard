export type VSCodeApi = ReturnType<typeof acquireVsCodeApi>;

let api: VSCodeApi | null = null;

export function getVsCodeApi(): VSCodeApi {
    if (!api) {
        api = acquireVsCodeApi();
    }
    return api;
}

export function getWebviewState<T>(fallback: T): T {
    try {
        const value = getVsCodeApi().getState();
        return (value as T) ?? fallback;
    } catch {
        return fallback;
    }
}

export function setWebviewState<T>(value: T): void {
    try {
        getVsCodeApi().setState(value);
    } catch {
        // ignore
    }
}

