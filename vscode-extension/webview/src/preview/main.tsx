import { render } from 'preact';
import { useSignal } from '@preact/signals';
import { useEffect, useMemo, useRef } from 'preact/hooks';
import MarkdownIt from 'markdown-it';
import taskLists from 'markdown-it-task-lists';
import footnote from 'markdown-it-footnote';
import emoji from 'markdown-it-emoji';
import hljs from 'highlight.js/lib/common';
import mermaid from 'mermaid';
import { applyTheme, coerceTheme, type UiTheme } from '../shared/theme';
import { getVsCodeApi, getWebviewState, setWebviewState } from '../shared/vscode';
import { Button, Badge } from '../components';

type PreviewData = {
    filePath: string;
    fileName: string;
    baseUri: string;
    markdown: string;
    updatedAt?: string | null;
};

type TocItem = {
    level: number;
    id: string;
    title: string;
};

type UiState = {
    theme: UiTheme;
    highlight: boolean;
    mermaid: boolean;
    wrap: boolean;
    showToc: boolean;
    followActive: boolean;
    outlineCollapsed: boolean;
};

const vscode = getVsCodeApi();

function getInitialTheme(): UiTheme {
    const init = window.__TONEGUARD_PREVIEW_INITIAL_STATE__;
    if (init && typeof init === 'object') {
        return coerceTheme((init as any).uiTheme);
    }
    return 'vscode';
}

function getInitialUiState(): UiState {
    const fallback: UiState = {
        theme: getInitialTheme(),
        highlight: true,
        mermaid: true,
        wrap: true,
        showToc: true,
        followActive: true,
        outlineCollapsed: false,
    };
    const stored = getWebviewState<Partial<UiState>>({});
    return {
        ...fallback,
        ...stored,
        theme: fallback.theme,
    };
}

function slugifyHeading(input: string, counts: Record<string, number>): string {
    const base = input
        .toLowerCase()
        .replace(/[^a-z0-9\s-]/g, '')
        .trim()
        .replace(/\s+/g, '-')
        .replace(/-+/g, '-');
    const core = base || 'section';
    const seen = (counts[core] ?? 0) + 1;
    counts[core] = seen;
    return seen > 1 ? `${core}-${seen}` : core;
}

function escapeHtml(value: string): string {
    return value
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
}

function escapeXml(value: string): string {
    return escapeHtml(value);
}

function escapeCdata(value: string): string {
    return value.replace(/]]>/g, ']]]]><![CDATA[>');
}

function isExternalUrl(value: string): boolean {
    return /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value);
}

function resolveRelativeUrl(value: string, baseUri: string): string {
    if (!value) return value;
    if (isExternalUrl(value) || value.startsWith('#')) {
        return value;
    }
    try {
        const base = baseUri.endsWith('/') ? baseUri : `${baseUri}/`;
        return new URL(value, base).toString();
    } catch {
        return value;
    }
}

function highlightCode(code: string, lang: string): string {
    const language = lang?.toLowerCase() || '';
    if (language && hljs.getLanguage(language)) {
        return hljs.highlight(code, { language }).value;
    }
    return hljs.highlightAuto(code).value;
}

function createMarkdownIt(options: {
    enableMermaid: boolean;
    enableHighlight: boolean;
    baseUri: string;
}): any {
    const md = new MarkdownIt({
        html: true,
        linkify: true,
        breaks: false,
        highlight: options.enableHighlight
            ? (code: string, lang: string) => highlightCode(code, lang || '')
            : undefined,
    });

    md.use(taskLists, { enabled: true, label: false });
    md.use(footnote);
    md.use(emoji);

    const baseFence =
        md.renderer.rules.fence ??
        ((tokens: any, idx: number, opts: any, env: any, self: any) =>
            self.renderToken(tokens, idx, opts));

    md.renderer.rules.fence = (tokens: any, idx: number, opts: any, env: any, self: any) => {
        const token = tokens[idx];
        const info = (token.info || '').trim().split(/\s+/)[0]?.toLowerCase() || '';
        if (info === 'mermaid' && options.enableMermaid) {
            const raw = md.utils.escapeHtml(token.content);
            return `
<div class="tg-mermaid-block">
    <div class="tg-mermaid-toolbar">
        <button class="tg-mermaid-btn" data-action="copy-mermaid" type="button">Copy Mermaid</button>
        <button class="tg-mermaid-btn" data-action="export-mermaid" type="button">Export SVG</button>
    </div>
    <div class="mermaid">${raw}</div>
</div>
`;
        }
        return baseFence(tokens, idx, opts, env, self);
    };

    const baseImage = md.renderer.rules.image;
    md.renderer.rules.image = (tokens: any, idx: number, opts: any, env: any, self: any) => {
        const token = tokens[idx];
        const src = token.attrGet('src') || '';
        const resolved = resolveRelativeUrl(src, options.baseUri);
        token.attrSet('src', resolved);
        if (baseImage) {
            return baseImage(tokens, idx, opts, env, self);
        }
        return self.renderToken(tokens, idx, opts);
    };

    md.renderer.rules.heading_open = (tokens: any, idx: number, opts: any, env: any, self: any) => {
        const heading = tokens[idx];
        const title = tokens[idx + 1]?.content ?? '';
        const level = Number(heading.tag?.slice(1) ?? '1');
        const state = env as { toc?: TocItem[]; slugCounts?: Record<string, number> };
        const counts = state.slugCounts ?? {};
        const id = slugifyHeading(title, counts);
        heading.attrSet('id', id);
        state.slugCounts = counts;
        if (!state.toc) state.toc = [];
        state.toc.push({ level, id, title });
        return self.renderToken(tokens, idx, opts);
    };

    return md;
}

function confluenceCodeMacro(code: string, language: string): string {
    const safe = escapeCdata(code);
    const langParam = language
        ? `<ac:parameter ac:name="language">${escapeXml(language)}</ac:parameter>`
        : '';
    return `
<ac:structured-macro ac:name="code">${langParam}<ac:plain-text-body><![CDATA[${safe}]]></ac:plain-text-body></ac:structured-macro>
`;
}

function createConfluenceMarkdownIt(): any {
    const md = new MarkdownIt({
        html: false,
        linkify: true,
        breaks: false,
    });

    md.use(taskLists, { enabled: true, label: false });
    md.use(footnote);
    md.use(emoji);

    md.renderer.rules.fence = (tokens: any, idx: number) => {
        const token = tokens[idx];
        const info = (token.info || '').trim().split(/\s+/)[0]?.toLowerCase() || '';
        return confluenceCodeMacro(token.content, info);
    };

    md.renderer.rules.code_block = (tokens: any, idx: number) => {
        const token = tokens[idx];
        return confluenceCodeMacro(token.content, '');
    };

    md.renderer.rules.image = (tokens: any, idx: number) => {
        const token = tokens[idx];
        const src = token.attrGet('src') || '';
        const alt = token.content || token.attrGet('alt') || '';
        if (isExternalUrl(src)) {
            return `<ac:image ac:alt="${escapeXml(alt)}"><ri:url ri:value="${escapeXml(src)}" /></ac:image>`;
        }
        const filename = src.split('/').pop() || src;
        return `<ac:image ac:alt="${escapeXml(alt)}"><ri:attachment ri:filename="${escapeXml(filename)}" /></ac:image>`;
    };

    return md;
}

function buildHtmlDocument(bodyHtml: string, title: string): string {
    const safeTitle = escapeHtml(title || 'ToneGuard Preview');
    const css = `
:root { color-scheme: light dark; }
body { margin: 0; padding: 24px; font-family: ui-serif, Georgia, 'Times New Roman', serif; line-height: 1.7; color: #1b1b1b; background: #ffffff; }
@media (prefers-color-scheme: dark) { body { color: #e6e6e6; background: #0f1115; } }
img { max-width: 100%; }
pre { background: #f5f5f7; padding: 12px 14px; border-radius: 8px; overflow: auto; }
code { font-family: ui-monospace, 'SF Mono', Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace; font-size: 0.92em; }
blockquote { margin: 16px 0; padding: 12px 16px; border-left: 3px solid #c9ccd4; background: #f8f9fb; }
hr { border: none; border-top: 1px solid #d7d7d7; margin: 24px 0; }
`;
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${safeTitle}</title>
<style>${css}</style>
</head>
<body>
${bodyHtml}
</body>
</html>`;
}

function buildFileName(value: string, fallback: string): string {
    const base = value
        .toLowerCase()
        .replace(/[^a-z0-9\s-]/g, '')
        .trim()
        .replace(/\s+/g, '-')
        .replace(/-+/g, '-');
    return base || fallback;
}

async function renderMermaidSvg(code: string): Promise<string> {
    const id = `tg-mermaid-${Math.random().toString(36).slice(2)}`;
    const api: any = mermaid;
    if (typeof api.render === 'function') {
        const result = await api.render(id, code);
        if (typeof result === 'string') {
            return result;
        }
        if (result && typeof result.svg === 'string') {
            return result.svg;
        }
    }
    if (api.mermaidAPI && typeof api.mermaidAPI.render === 'function') {
        return await new Promise((resolve, reject) => {
            api.mermaidAPI.render(
                id,
                code,
                (svg: string) => resolve(svg),
                (err: Error) => reject(err)
            );
        });
    }
    throw new Error('Mermaid render API not available');
}

function App() {
    const ui = useSignal<UiState>(getInitialUiState());
    const markdownText = useSignal('');
    const filePath = useSignal<string | null>(null);
    const fileName = useSignal('');
    const baseUri = useSignal('');
    const updatedAt = useSignal<string | null>(null);
    const html = useSignal('');
    const toc = useSignal<TocItem[]>([]);
    const error = useSignal<string | null>(null);
    const status = useSignal<string | null>(null);
    const previewRef = useRef<HTMLDivElement>(null);
    const fileMenuOpen = useSignal(false);
    const viewMenuOpen = useSignal(false);
    const exportMenuOpen = useSignal(false);

    const confluenceMd = useMemo(() => createConfluenceMarkdownIt(), []);

    useEffect(() => {
        applyTheme(ui.value.theme);
        setWebviewState({
            highlight: ui.value.highlight,
            mermaid: ui.value.mermaid,
            wrap: ui.value.wrap,
            showToc: ui.value.showToc,
            followActive: ui.value.followActive,
            outlineCollapsed: ui.value.outlineCollapsed,
        });
    }, [ui.value]);

    useEffect(() => {
        vscode.postMessage({ command: 'followActive', value: ui.value.followActive });
    }, [ui.value.followActive]);

    useEffect(() => {
        vscode.postMessage({ command: 'ready' });
    }, []);

    useEffect(() => {
        const handler = (event: MouseEvent) => {
            const target = event.target as HTMLElement | null;
            if (target && target.closest('.tg-dropdown')) {
                return;
            }
            if (fileMenuOpen.value || viewMenuOpen.value || exportMenuOpen.value) {
                fileMenuOpen.value = false;
                viewMenuOpen.value = false;
                exportMenuOpen.value = false;
            }
        };
        document.addEventListener('click', handler);
        return () => document.removeEventListener('click', handler);
    }, []);

    useEffect(() => {
        const handler = (event: MessageEvent) => {
            const message = event.data;
            if (!message || typeof message !== 'object') return;
            if (message.type === 'markdownData') {
                const data = message.data as PreviewData;
                markdownText.value = data.markdown || '';
                filePath.value = data.filePath || null;
                fileName.value = data.fileName || '';
                baseUri.value = data.baseUri || '';
                updatedAt.value = data.updatedAt ?? null;
                status.value = null;
                return;
            }
            if (message.type === 'theme') {
                ui.value = { ...ui.value, theme: coerceTheme(message.value) };
                return;
            }
            if (message.type === 'error') {
                error.value = String(message.message || 'Preview error');
                return;
            }
            if (message.type === 'status') {
                status.value = String(message.message || '');
            }
        };
        window.addEventListener('message', handler);
        return () => window.removeEventListener('message', handler);
    }, []);

    useEffect(() => {
        if (!markdownText.value) {
            html.value = '';
            toc.value = [];
            return;
        }
        try {
            const md = createMarkdownIt({
                enableMermaid: ui.value.mermaid,
                enableHighlight: ui.value.highlight,
                baseUri: baseUri.value,
            });
            const env: { toc?: TocItem[]; slugCounts?: Record<string, number> } = {};
            const tokens = md.parse(markdownText.value, env);
            html.value = md.renderer.render(tokens, md.options, env);
            toc.value = env.toc || [];
            error.value = null;
        } catch (err) {
            error.value = err instanceof Error ? err.message : String(err);
        }
    }, [markdownText.value, baseUri.value, ui.value.mermaid, ui.value.highlight]);

    useEffect(() => {
        if (!ui.value.mermaid) return;
        const root = previewRef.current;
        if (!root) return;
        const nodes = Array.from(root.querySelectorAll<HTMLElement>('.mermaid'));
        if (nodes.length === 0) return;
        for (const node of nodes) {
            node.removeAttribute('data-processed');
        }
        try {
            const preferDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
            mermaid.initialize({
                startOnLoad: false,
                securityLevel: 'strict',
                theme: preferDark ? 'dark' : 'default',
            });
            void mermaid.run({ nodes });
        } catch (err) {
            error.value = err instanceof Error ? err.message : String(err);
        }
    }, [html.value, ui.value.mermaid, ui.value.theme]);

    const handlePreviewClick = async (event: MouseEvent) => {
        const target = event.target as HTMLElement;
        const actionButton = target.closest('button[data-action]') as HTMLElement | null;
        if (actionButton) {
            const action = actionButton.dataset.action;
            const block = actionButton.closest('.tg-mermaid-block');
            const code = block?.querySelector('.mermaid')?.textContent || '';
            if (action === 'copy-mermaid' && code) {
                vscode.postMessage({ command: 'copyText', text: code, label: 'Mermaid' });
            }
            if (action === 'export-mermaid' && code) {
                try {
                    const svg = await renderMermaidSvg(code);
                    const name = buildFileName(fileName.value || 'diagram', 'diagram');
                    vscode.postMessage({
                        command: 'saveFile',
                        format: 'svg',
                        content: svg,
                        suggestedName: `${name}.svg`,
                    });
                } catch (err) {
                    const msg = err instanceof Error ? err.message : String(err);
                    vscode.postMessage({
                        command: 'showError',
                        message: `Mermaid export failed: ${msg}`,
                    });
                }
            }
            return;
        }

        const link = target.closest('a') as HTMLAnchorElement | null;
        if (link) {
            const href = link.getAttribute('href') || '';
            if (!href || href.startsWith('#')) {
                return;
            }
            event.preventDefault();
            if (isExternalUrl(href)) {
                vscode.postMessage({ command: 'openExternal', url: href });
            } else {
                vscode.postMessage({ command: 'openFileRelative', href });
            }
        }
    };

    const saveHtml = () => {
        const name = buildFileName(fileName.value || 'document', 'document');
        const doc = buildHtmlDocument(html.value, fileName.value || 'ToneGuard Preview');
        vscode.postMessage({
            command: 'saveFile',
            format: 'html',
            content: doc,
            suggestedName: `${name}.html`,
        });
    };

    const copyHtml = () => {
        vscode.postMessage({
            command: 'copyText',
            text: html.value,
            label: 'HTML',
        });
    };

    const exportConfluence = () => {
        const name = buildFileName(fileName.value || 'document', 'document');
        const out = confluenceMd.render(markdownText.value || '');
        vscode.postMessage({
            command: 'saveFile',
            format: 'confluence',
            content: out,
            suggestedName: `${name}.confluence.xml`,
        });
    };

    const copyConfluence = () => {
        const out = confluenceMd.render(markdownText.value || '');
        vscode.postMessage({
            command: 'copyText',
            text: out,
            label: 'Confluence storage',
        });
    };

    const printPdf = () => {
        window.print();
    };

    const updateUi = (patch: Partial<UiState>) => {
        ui.value = { ...ui.value, ...patch };
    };

    return (
        <div class="tg-preview">
            <div class="tg-preview-toolbar">
                <div class="tg-preview-toolbar-left">
                    <div class="tg-preview-actions">
                        <div class="tg-dropdown">
                            <Button
                                size="sm"
                                className="tg-toolbar-btn"
                                onClick={(event) => {
                                    event.stopPropagation();
                                    fileMenuOpen.value = !fileMenuOpen.value;
                                    viewMenuOpen.value = false;
                                    exportMenuOpen.value = false;
                                }}
                            >
                                File
                            </Button>
                            {fileMenuOpen.value && (
                                <div class="tg-dropdown-panel">
                                    <div class="tg-dropdown-title">File</div>
                                    <button
                                        class="tg-dropdown-action"
                                        type="button"
                                        onClick={() => {
                                            fileMenuOpen.value = false;
                                            vscode.postMessage({ command: 'selectFile' });
                                        }}
                                    >
                                        Choose file
                                    </button>
                                    <button
                                        class="tg-dropdown-action"
                                        type="button"
                                        disabled={!filePath.value}
                                        onClick={() => {
                                            fileMenuOpen.value = false;
                                            if (filePath.value) {
                                                vscode.postMessage({ command: 'openFile', path: filePath.value });
                                            }
                                        }}
                                    >
                                        Open in editor
                                    </button>
                                </div>
                            )}
                        </div>
                        <Button size="sm" className="tg-toolbar-btn" onClick={() => vscode.postMessage({ command: 'refresh' })}>
                            Refresh
                        </Button>
                    </div>
                    <div class="tg-preview-actions">
                        <div class="tg-dropdown">
                            <Button
                                size="sm"
                                className="tg-toolbar-btn"
                                onClick={(event) => {
                                    event.stopPropagation();
                                    fileMenuOpen.value = false;
                                    viewMenuOpen.value = !viewMenuOpen.value;
                                    exportMenuOpen.value = false;
                                }}
                            >
                                View
                            </Button>
                            {viewMenuOpen.value && (
                                <div class="tg-dropdown-panel">
                                    <div class="tg-dropdown-title">View options</div>
                                    <label class="tg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={ui.value.followActive}
                                            onChange={() => updateUi({ followActive: !ui.value.followActive })}
                                        />
                                        Follow active file
                                    </label>
                                    <label class="tg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={ui.value.highlight}
                                            onChange={() => updateUi({ highlight: !ui.value.highlight })}
                                        />
                                        Syntax highlight
                                    </label>
                                    <label class="tg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={ui.value.mermaid}
                                            onChange={() => updateUi({ mermaid: !ui.value.mermaid })}
                                        />
                                        Mermaid render
                                    </label>
                                    <label class="tg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={ui.value.wrap}
                                            onChange={() => updateUi({ wrap: !ui.value.wrap })}
                                        />
                                        Wrap code blocks
                                    </label>
                                    <label class="tg-toggle">
                                        <input
                                            type="checkbox"
                                            checked={ui.value.showToc}
                                            onChange={() => updateUi({ showToc: !ui.value.showToc })}
                                        />
                                        Show outline
                                    </label>
                                </div>
                            )}
                        </div>
                        <div class="tg-dropdown">
                            <Button
                                size="sm"
                                className="tg-toolbar-btn"
                                onClick={(event) => {
                                    event.stopPropagation();
                                    fileMenuOpen.value = false;
                                    exportMenuOpen.value = !exportMenuOpen.value;
                                    viewMenuOpen.value = false;
                                }}
                            >
                                Export
                            </Button>
                            {exportMenuOpen.value && (
                                <div class="tg-dropdown-panel tg-dropdown-panel-right">
                                    <div class="tg-dropdown-title">Export</div>
                                    <button class="tg-dropdown-action" type="button" onClick={copyHtml}>
                                        Copy HTML
                                    </button>
                                    <button class="tg-dropdown-action" type="button" onClick={saveHtml}>
                                        Export HTML
                                    </button>
                                    <button class="tg-dropdown-action" type="button" onClick={copyConfluence}>
                                        Copy Confluence
                                    </button>
                                    <button class="tg-dropdown-action" type="button" onClick={exportConfluence}>
                                        Export Confluence
                                    </button>
                                    <button class="tg-dropdown-action" type="button" onClick={printPdf}>
                                        Export PDF
                                    </button>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                {(fileName.value || filePath.value) && (
                    <div class="tg-preview-filemeta">
                        {fileName.value && <div class="tg-preview-filemeta-name">{fileName.value}</div>}
                        {filePath.value && (
                            <div class="tg-preview-filemeta-path" title={filePath.value}>
                                {filePath.value}
                            </div>
                        )}
                    </div>
                )}
            </div>

            <div class={`tg-preview-body ${ui.value.showToc ? '' : 'no-outline'}`}>
                {ui.value.showToc && (
                    <aside class="tg-preview-sidebar">
                        <div class="tg-preview-outline">
                            <div class="tg-preview-outline-header">
                                <span class="tg-preview-section-title">Outline</span>
                                <button
                                    class="tg-outline-toggle"
                                    type="button"
                                    onClick={() => updateUi({ outlineCollapsed: !ui.value.outlineCollapsed })}
                                >
                                    {ui.value.outlineCollapsed ? 'Show' : 'Hide'}
                                </button>
                            </div>
                            {!ui.value.outlineCollapsed && (
                                toc.value.length === 0 ? (
                                    <div class="tg-preview-subtle">No outline</div>
                                ) : (
                                    toc.value.map((item) => (
                                        <a
                                            key={item.id}
                                            class={`tg-preview-outline-item level-${item.level}`}
                                            href={`#${item.id}`}
                                        >
                                            {item.title || '(untitled)'}
                                        </a>
                                    ))
                                )
                            )}
                        </div>
                    </aside>
                )}

                <main class="tg-preview-main">
                    {error.value && (
                        <div class="tg-preview-alert" role="alert">
                            {error.value}
                        </div>
                    )}
                    {markdownText.value.length === 0 ? (
                        <div class="tg-preview-empty">
                            <Badge variant="default">No markdown loaded</Badge>
                            <p>
                                {status.value
                                    ? status.value
                                    : 'Select a file or use Follow to pick the active editor.'}
                            </p>
                        </div>
                    ) : (
                        <div
                            ref={previewRef}
                            class={`tg-md ${ui.value.wrap ? 'tg-md-wrap' : 'tg-md-nowrap'}`}
                            onClick={handlePreviewClick}
                            dangerouslySetInnerHTML={{ __html: html.value }}
                        />
                    )}
                </main>
            </div>
        </div>
    );
}

render(<App />, document.getElementById('app')!);
