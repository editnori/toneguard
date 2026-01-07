export type UiTheme = 'vscode' | 'maple-light';

export function coerceTheme(value: unknown): UiTheme {
    return value === 'maple-light' ? 'maple-light' : 'vscode';
}

export function applyTheme(theme: UiTheme): void {
    if (theme === 'maple-light') {
        document.documentElement.dataset.theme = 'maple-light';
        return;
    }
    delete document.documentElement.dataset.theme;
}

