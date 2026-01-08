declare module 'markdown-it';
declare module 'markdown-it-task-lists';
declare module 'markdown-it-footnote';
declare module 'markdown-it-emoji';

declare function acquireVsCodeApi(): {
    postMessage(message: unknown): void;
    getState(): unknown;
    setState(state: unknown): void;
};

interface Window {
    __TONEGUARD_INITIAL_STATE__?: unknown;
    __TONEGUARD_FLOWMAP_INITIAL_STATE__?: unknown;
    __TONEGUARD_PREVIEW_INITIAL_STATE__?: unknown;
}
