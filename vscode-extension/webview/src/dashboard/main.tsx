import { render } from 'preact';
import { useSignal, useComputed } from '@preact/signals';
import { useEffect } from 'preact/hooks';
import { applyTheme, coerceTheme, type UiTheme } from '../shared/theme';
import { getVsCodeApi, getWebviewState, setWebviewState } from '../shared/vscode';
import { Button, Card, CardHeader, CardTitle, CardContent, Tabs, TabPanel, Badge, Row, Stack, Input, Select, ListItem } from '../components';

// Types
type DetectedEnvironments = {
    cursor: boolean;
    claude: boolean;
    codex: boolean;
};

type DashboardState = {
    platform: string;
    configPath: string;
    configSource: 'workspace' | 'bundled' | 'custom' | 'missing';
    cli: { mode: 'bundled' | 'PATH' | 'missing'; path: string };
    lsp: { mode: 'bundled' | 'PATH'; path: string };
    binaryAvailable: boolean;
    binaryInstallCommand: string;
    flowsCount: number | null;
    lastAudit: { when: string; findings: number } | null;
    lastProposal: { when: string } | null;
    lastMarkdown: { when: string; findings: number } | null;
    lastBlueprint: { when: string; nodes: number; edges: number } | null;
    markdownSummary: {
        files: number;
        diagnostics: number;
        repoIssuesCount: number;
        repoIssues: { category: string; message: string; path: string }[];
        topFiles: { path: string; count: number; line?: number }[];
    } | null;
    markdownFindings: { path: string; line?: number; category: string; message: string; severity: string }[];
    repoFindings: { path: string; category: string; message: string }[];
    blueprintSummary: { nodes: number; edges: number; edgesResolved: number; errors: number } | null;
    flowSummary: {
        findings: number;
        byCategory: { category: string; count: number }[];
        topFindings: { path: string; line?: number; message: string; category?: string }[];
    } | null;
    flowFindings: { path: string; line?: number; category: string; message: string; severity: string }[];
    detectedEnvs: DetectedEnvironments;
    defaultAIEditor: 'auto' | 'cursor' | 'claude-code' | 'codex';
    uiTheme?: string;
    skills: {
        writing: { cursor: boolean; claude: boolean; codex: boolean };
        'logic-flow': { cursor: boolean; claude: boolean; codex: boolean };
    };
    profileOptions: string[];
    currentProfile: string;
    settings: { strict: boolean; noRepoChecks: boolean };
    configEditable: boolean;
    enabledCategories: string[];
    ignoreGlobs: string[];
    enabledFileTypes: string[];
};

type UiState = {
    tab: 'review' | 'findings' | 'organize' | 'settings';
    theme: UiTheme;
};

const vscode = getVsCodeApi();

function getInitialDashboardState(): DashboardState {
    const fromWindow = window.__TONEGUARD_INITIAL_STATE__;
    if (!fromWindow || typeof fromWindow !== 'object') {
        return {
            platform: 'unknown',
            configPath: '<unset>',
            configSource: 'missing',
            cli: { mode: 'missing', path: 'dwg' },
            lsp: { mode: 'PATH', path: 'dwg-lsp' },
            binaryAvailable: false,
            binaryInstallCommand: '',
            flowsCount: null,
            lastAudit: null,
            lastProposal: null,
            lastMarkdown: null,
            lastBlueprint: null,
            markdownSummary: null,
            markdownFindings: [],
            repoFindings: [],
            flowSummary: null,
            flowFindings: [],
            blueprintSummary: null,
            detectedEnvs: { cursor: false, claude: false, codex: false },
            defaultAIEditor: 'auto',
            skills: {
                writing: { cursor: false, claude: false, codex: false },
                'logic-flow': { cursor: false, claude: false, codex: false },
            },
            profileOptions: [],
            currentProfile: '',
            settings: { strict: false, noRepoChecks: false },
            configEditable: false,
            enabledCategories: [],
            ignoreGlobs: [],
            enabledFileTypes: [],
        };
    }

    const raw = fromWindow as any;
    return {
        ...(raw as DashboardState),
        markdownFindings: Array.isArray(raw?.markdownFindings) ? raw.markdownFindings : [],
        repoFindings: Array.isArray(raw?.repoFindings) ? raw.repoFindings : [],
        flowFindings: Array.isArray(raw?.flowFindings) ? raw.flowFindings : [],
    };
}

// StatusBadge component
function StatusBadge({ label, variant }: { label: string; variant: 'success' | 'warning' | 'default' }) {
    return <Badge variant={variant}>{label}</Badge>;
}

// Stat row component
function StatRow({ label, value }: { label: string; value: string }) {
    return (
        <div className="flex items-center justify-between text-[11px]">
            <span className="text-muted">{label}</span>
            <span className="text-text">{value}</span>
        </div>
    );
}

function includesQuery(haystack: string, query: string): boolean {
    if (!query) return true;
    return haystack.toLowerCase().includes(query);
}

// Review Tab
function ReviewTab({ state }: { state: DashboardState }) {
    const lastAudit = state.lastAudit
        ? `${state.lastAudit.findings} findings · ${state.lastAudit.when}`
        : '—';
    const lastProposal = state.lastProposal ? state.lastProposal.when : '—';
    const lastMarkdown = state.lastMarkdown
        ? `${state.lastMarkdown.findings} issues · ${state.lastMarkdown.when}`
        : '—';
    const lastBlueprint = state.lastBlueprint
        ? `${state.lastBlueprint.nodes}n · ${state.lastBlueprint.edges}e`
        : '—';

    return (
        <Stack gap="sm">
            {/* Status Card */}
            <Card padding="sm">
                <Row justify="between" gap="xs">
                    <span className="text-[11px] font-semibold">Status</span>
                    <Row gap="xs">
                        <StatusBadge 
                            label={state.binaryAvailable ? (state.cli.mode === 'bundled' ? 'Bundled' : 'PATH') : 'Missing'} 
                            variant={state.binaryAvailable ? 'success' : 'warning'} 
                        />
                        <StatusBadge 
                            label={state.configSource === 'workspace' ? 'Workspace' : state.configSource} 
                            variant={state.configSource === 'workspace' ? 'success' : 'warning'} 
                        />
                    </Row>
                </Row>
                <div className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1">
                    <StatRow label="Audit" value={lastAudit} />
                    <StatRow label="Proposal" value={lastProposal} />
                    <StatRow label="Markdown" value={lastMarkdown} />
                    <StatRow label="Blueprint" value={lastBlueprint} />
                    <StatRow label="Flows" value={state.flowsCount === null ? '—' : String(state.flowsCount)} />
                </div>
                <Row gap="xs" className="mt-2 flex-wrap">
                    <Button size="sm" variant="primary" onClick={() => vscode.postMessage({ type: 'runRecommended' })}>
                        Run
                    </Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'runAudit' })}>Audit</Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'runProposal' })}>Proposal</Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'newFlow' })}>New flow</Button>
                    <Button size="sm" variant="ghost" onClick={() => vscode.postMessage({ type: 'copyReviewBundle' })}>
                        Copy bundle
                    </Button>
                </Row>
            </Card>

            {/* Markdown Card */}
            {state.markdownSummary && (state.markdownSummary.diagnostics > 0 || state.markdownSummary.repoIssuesCount > 0) && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Markdown</span>
                        <span className="text-[10px] text-muted">
                            {state.markdownSummary.diagnostics} file · {state.markdownSummary.repoIssuesCount} repo
                        </span>
                    </Row>
                    <div className="mt-1.5 space-y-0.5">
                        {state.markdownSummary.topFiles.slice(0, 5).map((f, i) => (
                            <ListItem
                                key={i}
                                onClick={() => vscode.postMessage({
                                    type: 'openMarkdownFile',
                                    path: f.path,
                                    line: f.line,
                                })}
                            >
                                <span className="text-muted mr-1">{f.count}</span>
                                {f.path}{f.line ? `:${f.line}` : ''}
                            </ListItem>
                        ))}
                    </div>
                    <Row gap="xs" className="mt-2">
                        <Button size="sm" onClick={() => vscode.postMessage({ type: 'openMarkdownReport' })}>
                            Report
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => vscode.postMessage({ type: 'runRecommended' })}>
                            Re-run
                        </Button>
                    </Row>
                </Card>
            )}

            {/* Flow Card */}
            {state.flowSummary && state.flowSummary.findings > 0 && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Flow</span>
                        <span className="text-[10px] text-muted">{state.flowSummary.findings} findings</span>
                    </Row>
                    <div className="mt-1.5 space-y-0.5">
                        {state.flowSummary.topFindings.slice(0, 4).map((f, i) => (
                            <ListItem
                                key={i}
                                onClick={() => vscode.postMessage({
                                    type: 'openFlowFinding',
                                    path: f.path,
                                    line: f.line,
                                })}
                            >
                                <span className="text-muted mr-1">{f.category ?? 'issue'}</span>
                                {f.path}{f.line ? `:${f.line}` : ''} · {f.message.slice(0, 40)}
                            </ListItem>
                        ))}
                    </div>
                    <Row gap="xs" className="mt-2">
                        <Button size="sm" onClick={() => vscode.postMessage({ type: 'openFlowAudit' })}>Audit</Button>
                        <Button size="sm" onClick={() => vscode.postMessage({ type: 'openFlowMap' })}>Flow Map</Button>
                    </Row>
                </Card>
            )}

            {/* Blueprint Card */}
            {state.blueprintSummary && state.blueprintSummary.edges > 0 && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Blueprint</span>
                        <span className="text-[10px] text-muted">
                            {state.blueprintSummary.nodes}n · {state.blueprintSummary.edges}e
                        </span>
                    </Row>
                    <div className="mt-1 grid grid-cols-2 gap-1 text-[10px]">
                        <StatRow label="Resolved" value={String(state.blueprintSummary.edgesResolved)} />
                        <StatRow label="Errors" value={String(state.blueprintSummary.errors)} />
                    </div>
                    <Row gap="xs" className="mt-2">
                        <Button size="sm" onClick={() => vscode.postMessage({ type: 'openFlowMap' })}>Flow Map</Button>
                        <Button size="sm" onClick={() => vscode.postMessage({ type: 'runBlueprint' })}>Rebuild</Button>
                        <Button size="sm" variant="ghost" onClick={() => vscode.postMessage({ type: 'openBlueprintDiffReport' })}>
                            Diff
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => vscode.postMessage({ type: 'openBlueprintMapping' })}>
                            Mapping
                        </Button>
                    </Row>
                </Card>
            )}
        </Stack>
    );
}

// Findings Tab
function FindingsTab({ state }: { state: DashboardState }) {
    const query = useSignal('');
    const q = useComputed(() => query.value.trim().toLowerCase());

    const md = useComputed(() => {
        return (state.markdownFindings || []).filter((f) =>
            includesQuery(`${f.category} ${f.path} ${f.message}`, q.value)
        );
    });

    const repo = useComputed(() => {
        return (state.repoFindings || []).filter((f) =>
            includesQuery(`${f.category} ${f.path} ${f.message}`, q.value)
        );
    });

    const flow = useComputed(() => {
        return (state.flowFindings || []).filter((f) =>
            includesQuery(`${f.category} ${f.path} ${f.message}`, q.value)
        );
    });

    return (
        <Stack gap="sm">
            <Card padding="sm">
                <Row justify="between">
                    <span className="text-[11px] font-semibold">Findings</span>
                    <span className="text-[10px] text-muted">
                        {flow.value.length} flow · {md.value.length} md · {repo.value.length} repo
                    </span>
                </Row>
                <Row gap="xs" className="mt-2">
                    <Input
                        size="sm"
                        className="flex-1"
                        placeholder="Filter (path, category, text)…"
                        value={query.value}
                        onInput={(e) => (query.value = (e.target as HTMLInputElement).value)}
                    />
                    <Button size="sm" variant="ghost" onClick={() => (query.value = '')}>
                        Clear
                    </Button>
                </Row>
            </Card>

            {md.value.length > 0 && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Markdown</span>
                        <Row gap="xs">
                            <Button size="sm" onClick={() => vscode.postMessage({ type: 'openMarkdownReport' })}>
                                Report
                            </Button>
                            <Button size="sm" variant="ghost" onClick={() => vscode.postMessage({ type: 'runRecommended' })}>
                                Re-run
                            </Button>
                        </Row>
                    </Row>
                    <div className="mt-1.5 space-y-0.5 max-h-[30vh] overflow-auto">
                        {md.value.slice(0, 200).map((f, i) => (
                            <ListItem
                                key={i}
                                onClick={() => vscode.postMessage({
                                    type: 'openMarkdownFile',
                                    path: f.path,
                                    line: f.line,
                                })}
                            >
                                <span className="text-muted mr-1">{f.category}</span>
                                {f.path}{f.line ? `:${f.line}` : ''} · {f.message}
                            </ListItem>
                        ))}
                    </div>
                </Card>
            )}

            {repo.value.length > 0 && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Repo</span>
                        <span className="text-[10px] text-muted">{repo.value.length} issues</span>
                    </Row>
                    <div className="mt-1.5 space-y-0.5 max-h-[15vh] overflow-auto">
                        {repo.value.slice(0, 200).map((f, i) => (
                            <ListItem
                                key={i}
                                disabled={!f.path || f.path === '<repo>'}
                                onClick={() => {
                                    if (!f.path || f.path === '<repo>') {
                                        return;
                                    }
                                    vscode.postMessage({
                                        type: 'openMarkdownFile',
                                        path: f.path,
                                    });
                                }}
                            >
                                <span className="text-muted mr-1">{f.category}</span>
                                {f.path} · {f.message}
                            </ListItem>
                        ))}
                    </div>
                </Card>
            )}

            {flow.value.length > 0 && (
                <Card padding="sm">
                    <Row justify="between">
                        <span className="text-[11px] font-semibold">Flow</span>
                        <Row gap="xs">
                            <Button size="sm" onClick={() => vscode.postMessage({ type: 'openFlowAudit' })}>
                                Audit
                            </Button>
                            <Button size="sm" onClick={() => vscode.postMessage({ type: 'openFlowMap' })}>
                                Flow Map
                            </Button>
                        </Row>
                    </Row>
                    <div className="mt-1.5 space-y-0.5 max-h-[30vh] overflow-auto">
                        {flow.value.slice(0, 200).map((f, i) => (
                            <ListItem
                                key={i}
                                onClick={() => vscode.postMessage({
                                    type: 'openFlowFinding',
                                    path: f.path,
                                    line: f.line,
                                })}
                            >
                                <span className="text-muted mr-1">{f.category}</span>
                                {f.path}{f.line ? `:${f.line}` : ''} · {f.message}
                            </ListItem>
                        ))}
                    </div>
                </Card>
            )}
        </Stack>
    );
}

// Organize Tab
function OrganizeTab() {
    const dataMinKb = useSignal(100);
    const skipGit = useSignal(false);
    const result = useSignal<{ repoType: string; confidence: string; issueCount: number } | null>(null);
    const findings = useSignal<{ path: string; action: string; reason: string }[]>([]);

    useEffect(() => {
        const handler = (event: MessageEvent) => {
            const msg = event.data;
            if (msg?.type === 'organizeResult') {
                result.value = {
                    repoType: msg.repoType ?? 'Mixed',
                    confidence: msg.confidence ?? '—',
                    issueCount: msg.issueCount ?? 0,
                };
                findings.value = (msg.findings || []).slice(0, 20);
            }
        };
        window.addEventListener('message', handler);
        return () => window.removeEventListener('message', handler);
    }, []);

    return (
        <Stack gap="sm">
            <Card padding="sm">
                <span className="text-[11px] font-semibold">Organizer</span>
                <p className="text-[10px] text-muted mt-0.5">Find misplaced data, scripts, and legacy files</p>
                <Row gap="xs" className="mt-2">
                    <label className="text-[10px] text-muted">Min KB</label>
                    <Input
                        type="number"
                        size="sm"
                        className="w-14"
                        value={dataMinKb.value}
                        onInput={(e) => dataMinKb.value = Number((e.target as HTMLInputElement).value)}
                    />
                    <label className="flex items-center gap-1 text-[10px] text-muted ml-2">
                        <input
                            type="checkbox"
                            checked={skipGit.value}
                            onChange={(e) => skipGit.value = (e.target as HTMLInputElement).checked}
                        />
                        Skip git
                    </label>
                </Row>
                <Row gap="xs" className="mt-2 flex-wrap">
                    <Button size="sm" variant="primary" onClick={() => vscode.postMessage({
                        type: 'runOrganize',
                        dataMinKb: dataMinKb.value,
                        skipGit: skipGit.value,
                    })}>
                        Analyze
                    </Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'orgFixCursor' })}>Cursor</Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'orgFixClaude' })}>Claude</Button>
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'orgFixCodex' })}>Codex</Button>
                </Row>
                {result.value && (
                    <p className="text-[10px] text-muted mt-2">
                        {result.value.repoType} · {result.value.confidence} · {result.value.issueCount} issues
                    </p>
                )}
                {findings.value.length > 0 && (
                    <div className="mt-2 space-y-0.5">
                        {findings.value.map((f, i) => (
                            <div key={i} className="text-[10px] flex items-center gap-1">
                                <span className="text-muted uppercase">{f.action}</span>
                                <span className="text-text">{f.path}</span>
                                <span className="text-muted">· {f.reason}</span>
                            </div>
                        ))}
                    </div>
                )}
            </Card>
        </Stack>
    );
}

// Settings Tab
function SettingsTab({ state, theme, onThemeChange }: { 
    state: DashboardState; 
    theme: UiTheme;
    onThemeChange: (t: UiTheme) => void;
}) {
    const ignoreInput = useSignal('');

    return (
        <Stack gap="sm">
            {/* Appearance */}
            <Card padding="sm">
                <Row justify="between">
                    <span className="text-[11px] font-semibold">Appearance</span>
                    <Row gap="xs">
                        <label className="text-[10px] text-muted">Theme</label>
                        <Select
                            value={theme}
                            onChange={(e) => {
                                const val = (e.target as HTMLSelectElement).value as UiTheme;
                                onThemeChange(val);
                                vscode.postMessage({ type: 'setUiTheme', value: val });
                            }}
                        >
                            <option value="vscode">VS Code</option>
                            <option value="maple-light">Maple</option>
                        </Select>
                    </Row>
                </Row>
            </Card>

            {/* Config */}
            <Card padding="sm">
                <Row justify="between">
                    <span className="text-[11px] font-semibold">Config</span>
                    <span className="text-[10px] text-muted truncate max-w-[150px]">{state.configPath}</span>
                </Row>
                <Row gap="xs" className="mt-2 flex-wrap">
                    <Button size="sm" onClick={() => vscode.postMessage({ type: 'openConfig' })}>Open</Button>
                    <label className="flex items-center gap-1 text-[10px] text-muted">
                        <input
                            type="checkbox"
                            checked={state.settings.strict}
                            onChange={(e) => vscode.postMessage({
                                type: 'toggleSetting',
                                key: 'strict',
                                value: (e.target as HTMLInputElement).checked,
                            })}
                        />
                        Strict
                    </label>
                    <label className="flex items-center gap-1 text-[10px] text-muted">
                        <input
                            type="checkbox"
                            checked={state.settings.noRepoChecks}
                            onChange={(e) => vscode.postMessage({
                                type: 'toggleSetting',
                                key: 'noRepoChecks',
                                value: (e.target as HTMLInputElement).checked,
                            })}
                        />
                        Skip repo
                    </label>
                </Row>
            </Card>

            {/* Ignore Globs */}
            <Card padding="sm">
                <span className="text-[11px] font-semibold">Ignore globs</span>
                <div className="flex flex-wrap gap-1 mt-1.5">
                    {(state.ignoreGlobs || []).slice(0, 15).map((g, i) => (
                        <button
                            key={i}
                            className="px-1.5 py-0.5 text-[9px] border border-border rounded text-muted hover:border-danger hover:text-danger"
                            onClick={() => vscode.postMessage({ type: 'removeIgnore', glob: g })}
                        >
                            {g} ×
                        </button>
                    ))}
                    {(!state.ignoreGlobs || state.ignoreGlobs.length === 0) && (
                        <span className="text-[10px] text-muted">none</span>
                    )}
                </div>
                <Row gap="xs" className="mt-2">
                    <Input
                        size="sm"
                        placeholder="**/dist/**"
                        className="flex-1"
                        value={ignoreInput.value}
                        onInput={(e) => ignoreInput.value = (e.target as HTMLInputElement).value}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                                vscode.postMessage({ type: 'addIgnore', value: ignoreInput.value });
                                ignoreInput.value = '';
                            }
                        }}
                    />
                    <Button size="sm" onClick={() => {
                        vscode.postMessage({ type: 'addIgnore', value: ignoreInput.value });
                        ignoreInput.value = '';
                    }}>
                        Add
                    </Button>
                </Row>
            </Card>
        </Stack>
    );
}

// Main App
function App() {
    const initialState = getInitialDashboardState();
    const initialUi = getWebviewState<UiState>({ tab: 'review', theme: coerceTheme(initialState.uiTheme) });
    
    const state = useSignal<DashboardState>(initialState);
    const activeTab = useSignal<'review' | 'findings' | 'organize' | 'settings'>(initialUi.tab);
    const theme = useSignal<UiTheme>(initialUi.theme);

    // Apply theme on change
    useEffect(() => {
        applyTheme(theme.value);
        setWebviewState({ tab: activeTab.value, theme: theme.value });
    }, [theme.value, activeTab.value]);

    // Listen for state updates
    useEffect(() => {
        const handler = (event: MessageEvent) => {
            const msg = event.data;
            if (msg?.type === 'state' && msg.state) {
                state.value = msg.state as DashboardState;
            }
        };
        window.addEventListener('message', handler);
        return () => window.removeEventListener('message', handler);
    }, []);

    const tabs = [
        { id: 'review', label: 'Review' },
        { id: 'findings', label: 'Findings' },
        { id: 'organize', label: 'Organize' },
        { id: 'settings', label: 'Settings' },
    ];

    return (
        <div className="min-h-full p-2 text-[11px]">
            {/* Header */}
            <Row justify="between" className="mb-2">
                <span className="text-[12px] font-semibold text-text">ToneGuard</span>
                <span className="text-[10px] text-muted">{state.value.platform}</span>
            </Row>

            {/* Tabs */}
            <Tabs 
                tabs={tabs} 
                activeTab={activeTab as any}
                onChange={(id) => activeTab.value = id as 'review' | 'findings' | 'organize' | 'settings'}
            />

            {/* Tab Panels */}
            <TabPanel id="review" activeTab={activeTab as any}>
                <ReviewTab state={state.value} />
            </TabPanel>
            <TabPanel id="findings" activeTab={activeTab as any}>
                <FindingsTab state={state.value} />
            </TabPanel>
            <TabPanel id="organize" activeTab={activeTab as any}>
                <OrganizeTab />
            </TabPanel>
            <TabPanel id="settings" activeTab={activeTab as any}>
                <SettingsTab 
                    state={state.value} 
                    theme={theme.value}
                    onThemeChange={(t) => theme.value = t}
                />
            </TabPanel>

            <div className="mt-3 pt-2 border-t border-border text-[10px] text-muted">
                <button
                    className="text-accent hover:text-text"
                    onClick={() => vscode.postMessage({ type: 'openGithubFeedback' })}
                >
                    Please visit my GitHub for feedback
                </button>
            </div>
        </div>
    );
}

// Mount
const root = document.getElementById('app');
if (root) {
    render(<App />, root);
}
