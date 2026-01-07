import { render } from 'preact';
import { useSignal, useComputed } from '@preact/signals';
import { useEffect, useMemo } from 'preact/hooks';
import { applyTheme, coerceTheme, type UiTheme } from '../shared/theme';
import { getVsCodeApi, getWebviewState, setWebviewState } from '../shared/vscode';
import { Button, Card, Row, Stack, Input, Select, ListItem, Badge } from '../components';
import { Graph, type GraphNode, type GraphEdge } from '../components/Graph';

// Types
type FlowIndexItem = {
    display_name: string;
    target_name: string;
    file: string;
    file_display: string;
    start_line: number;
    language: string;
    kind: string;
};

type IndexData = {
    functions: number;
    files_scanned: number;
    by_language?: Record<string, number>;
    items: FlowIndexItem[];
};

type BlueprintNode = {
    path: string;
    abs_path: string;
    language: string;
    size_bytes: number;
    lines: number;
};

type BlueprintEdge = {
    from: string;
    to?: string | null;
    to_raw: string;
    kind: string;
    line?: number | null;
    resolved: boolean;
};

type BlueprintReport = {
    nodes: BlueprintNode[];
    edges: BlueprintEdge[];
    stats: {
        files_scanned: number;
        nodes: number;
        edges: number;
        edges_resolved: number;
        by_language?: Record<string, number>;
        by_edge_kind?: Record<string, number>;
    };
    errors: { path: string; message: string }[];
};

type CallgraphNode = {
    id: string;
    display_name: string;
    target_name: string;
    file: string;
    file_display: string;
    start_line: number;
    kind: string;
    language?: string;
    in_calls?: number;
    out_calls?: number;
    total_calls?: number;
};

type CallgraphEdge = {
    from: string;
    to?: string | null;
    to_raw: string;
    kind: string;
    line?: number | null;
    resolved: boolean;
};

type CallgraphReport = {
    nodes: CallgraphNode[];
    edges: CallgraphEdge[];
    stats: {
        files_scanned: number;
        nodes: number;
        edges: number;
        edges_resolved: number;
        orphan_nodes?: number;
        sink_nodes?: number;
        source_nodes?: number;
        by_language?: Record<string, number>;
        top_hubs?: { id: string; display_name: string; total_calls: number }[];
    };
    errors: { path: string; message: string }[];
};

type CfgItem = {
    name: string;
    file: string;
    start_line: number;
    language: string;
    nodes: number;
    edges: number;
    exits: number[];
    unreachable: number;
    mermaid?: string | null;
};

type GraphData = {
    cfgs: CfgItem[];
    logic_findings?: { category?: string; severity?: string; message?: string; path?: string; line?: number }[];
};

type UiState = {
    theme: UiTheme;
    search: string;
    mode: 'functions' | 'calls' | 'blueprint';
};

const vscode = getVsCodeApi();

function getInitialTheme(): UiTheme {
    const init = window.__TONEGUARD_FLOWMAP_INITIAL_STATE__;
    if (init && typeof init === 'object') {
        return coerceTheme((init as any).uiTheme);
    }
    return 'vscode';
}

const CLUSTER_COLORS = [
    '#0d7377',
    '#c45d35',
    '#2563eb',
    '#7c3aed',
    '#059669',
    '#d97706',
    '#be123c',
    '#0891b2',
    '#4f46e5',
    '#65a30d',
];

function clusterColor(id: number): string {
    return CLUSTER_COLORS[id % CLUSTER_COLORS.length];
}

type BlueprintDegrees = { in: number; out: number; total: number };

type BlueprintCluster = {
    id: number;
    nodes: string[];
    edges: number;
    byLanguage: Record<string, number>;
};

function normalizeEdgeKind(kind: string): string {
    const lower = String(kind || '').toLowerCase();
    if (lower === 'mod' || lower === 'use' || lower === 'import') {
        return lower;
    }
    if (lower.includes('import')) return 'import';
    if (lower.includes('mod')) return 'mod';
    if (lower.includes('use')) return 'use';
    return lower;
}

function computeBlueprintMeta(
    nodes: BlueprintNode[],
    edges: BlueprintEdge[],
    allowedKinds: Set<string>
): {
    resolvedEdges: BlueprintEdge[];
    degrees: Map<string, BlueprintDegrees>;
    nodeToCluster: Map<string, number>;
    clusters: BlueprintCluster[];
    hubs: { path: string; in: number; out: number; total: number }[];
    orphans: Set<string>;
    sinks: Set<string>;
} {
    const nodeSet = new Set(nodes.map((n) => n.path));
    const languageByPath = new Map(nodes.map((n) => [n.path, n.language] as const));

    const resolvedEdges = edges
        .filter((e) => e.resolved && e.to && allowedKinds.has(normalizeEdgeKind(e.kind)))
        .filter((e) => nodeSet.has(e.from) && nodeSet.has(e.to!));

    const degrees = new Map<string, BlueprintDegrees>();
    for (const n of nodes) {
        degrees.set(n.path, { in: 0, out: 0, total: 0 });
    }
    const adjacency = new Map<string, string[]>();
    for (const n of nodes) {
        adjacency.set(n.path, []);
    }

    for (const e of resolvedEdges) {
        const to = e.to!;
        const fromDeg = degrees.get(e.from);
        if (fromDeg) {
            fromDeg.out += 1;
            fromDeg.total += 1;
        }
        const toDeg = degrees.get(to);
        if (toDeg) {
            toDeg.in += 1;
            toDeg.total += 1;
        }
        adjacency.get(e.from)?.push(to);
        adjacency.get(to)?.push(e.from);
    }

    const nodeToCluster = new Map<string, number>();
    const visited = new Set<string>();
    const clusters: BlueprintCluster[] = [];
    let clusterId = 0;

    for (const n of nodes) {
        if (visited.has(n.path)) continue;
        const queue = [n.path];
        visited.add(n.path);
        const members: string[] = [];

        while (queue.length > 0) {
            const cur = queue.pop();
            if (!cur) break;
            members.push(cur);
            nodeToCluster.set(cur, clusterId);
            const neighbors = adjacency.get(cur) ?? [];
            for (const next of neighbors) {
                if (visited.has(next)) continue;
                visited.add(next);
                queue.push(next);
            }
        }

        const byLanguage: Record<string, number> = {};
        for (const m of members) {
            const lang = languageByPath.get(m) ?? 'unknown';
            byLanguage[lang] = (byLanguage[lang] ?? 0) + 1;
        }

        clusters.push({
            id: clusterId,
            nodes: members.sort(),
            edges: 0,
            byLanguage,
        });

        clusterId += 1;
    }

    for (const e of resolvedEdges) {
        const to = e.to!;
        const a = nodeToCluster.get(e.from);
        const b = nodeToCluster.get(to);
        if (a !== undefined && a === b) {
            const cluster = clusters[a];
            if (cluster) cluster.edges += 1;
        }
    }

    clusters.sort((a, b) => b.nodes.length - a.nodes.length);

    const hubs = nodes
        .map((n) => {
            const d = degrees.get(n.path) ?? { in: 0, out: 0, total: 0 };
            return { path: n.path, in: d.in, out: d.out, total: d.total };
        })
        .filter((h) => h.total > 0)
        .sort((a, b) => b.total - a.total)
        .slice(0, 20);

    const orphans = new Set<string>();
    const sinks = new Set<string>();
    for (const [path, d] of degrees.entries()) {
        if (d.in === 0) orphans.add(path);
        if (d.out === 0) sinks.add(path);
    }

    return { resolvedEdges, degrees, nodeToCluster, clusters, hubs, orphans, sinks };
}

// Sidebar List
function FileList({
    items,
    search,
    selectedId,
    onSelect,
}: {
    items: BlueprintNode[];
    search: string;
    selectedId: string;
    onSelect: (path: string) => void;
}) {
    const q = search.trim().toLowerCase();
    const filtered = q
        ? items.filter((it) => it.path.toLowerCase().includes(q) || it.language.toLowerCase().includes(q))
        : items;

    return (
        <div className="max-h-[50vh] overflow-auto space-y-0.5">
            {filtered.slice(0, 200).map((it) => (
                <ListItem
                    key={it.path}
                    active={selectedId === it.path}
                    onClick={() => onSelect(it.path)}
                >
                    <span className="text-muted mr-1">{it.language}</span>
                    {it.path.length > 40 ? '…' + it.path.slice(-38) : it.path}
                </ListItem>
            ))}
            {filtered.length === 0 && (
                <p className="text-[10px] text-muted px-2 py-1">No matches</p>
            )}
        </div>
    );
}

function FunctionList({
    items,
    search,
    selectedFile,
    selectedFn,
    onSelect,
}: {
    items: FlowIndexItem[];
    search: string;
    selectedFile: string;
    selectedFn: string;
    onSelect: (file: string, fn: string) => void;
}) {
    const q = search.trim().toLowerCase();
    const filtered = q
        ? items.filter((it) => it.display_name.toLowerCase().includes(q) || it.file_display.toLowerCase().includes(q))
        : items;

    return (
        <div className="max-h-[50vh] overflow-auto space-y-0.5">
            {filtered.slice(0, 200).map((it, i) => (
                <ListItem
                    key={`${it.file}-${it.target_name}-${i}`}
                    active={selectedFile === it.file && selectedFn === it.target_name}
                    onClick={() => onSelect(it.file, it.target_name)}
                >
                    <span className="text-muted mr-1">{it.kind}</span>
                    {it.display_name} · {it.file_display}:{it.start_line}
                </ListItem>
            ))}
            {filtered.length === 0 && (
                <p className="text-[10px] text-muted px-2 py-1">No matches</p>
            )}
        </div>
    );
}

function CallgraphList({
    items,
    search,
    selectedId,
    onSelect,
}: {
    items: CallgraphNode[];
    search: string;
    selectedId: string;
    onSelect: (id: string) => void;
}) {
    const q = search.trim().toLowerCase();
    const filtered = q
        ? items.filter((it) =>
            (it.display_name || '').toLowerCase().includes(q) ||
            (it.file_display || '').toLowerCase().includes(q)
        )
        : items;

    return (
        <div className="max-h-[50vh] overflow-auto space-y-0.5">
            {filtered.slice(0, 200).map((it) => (
                <ListItem
                    key={it.id}
                    active={selectedId === it.id}
                    onClick={() => onSelect(it.id)}
                >
                    <span className="text-muted mr-1">{it.language ?? 'code'}</span>
                    {it.display_name} · {it.file_display}:{it.start_line}
                </ListItem>
            ))}
            {filtered.length === 0 && (
                <p className="text-[10px] text-muted px-2 py-1">No matches</p>
            )}
        </div>
    );
}

// Edge list panel
function EdgeList({
    edges,
    selectedPath,
    nodes,
    allowedKinds,
}: {
    edges: BlueprintEdge[];
    selectedPath: string;
    nodes: Map<string, BlueprintNode>;
    allowedKinds: Set<string>;
}) {
    const outbound = edges
        .filter((e) => e.from === selectedPath)
        .filter((e) => allowedKinds.has(normalizeEdgeKind(e.kind)));
    const inbound = edges
        .filter((e) => e.to === selectedPath)
        .filter((e) => allowedKinds.has(normalizeEdgeKind(e.kind)));

    const renderEdge = (e: BlueprintEdge, dir: 'out' | 'in') => {
        const target = dir === 'out' ? (e.to ?? '') : e.from;
        const resolved = Boolean(e.resolved && target);
        const targetNode = resolved ? nodes.get(target) : undefined;
        const label = resolved ? target : e.to_raw;
        const selectedNode = nodes.get(selectedPath);
        const openFile = () => {
            if (dir === 'out') {
                if (selectedNode?.abs_path) {
                    vscode.postMessage({
                        command: 'openFile',
                        file: selectedNode.abs_path,
                        line: e.line ?? undefined,
                    });
                }
                return;
            }
            if (resolved && targetNode?.abs_path) {
                vscode.postMessage({
                    command: 'openFile',
                    file: targetNode.abs_path,
                    line: e.line ?? undefined,
                });
            }
        };

        return (
            <ListItem
                key={`${dir}-${e.from}-${e.to_raw}`}
                onClick={openFile}
            >
                <span className="text-muted uppercase mr-1">{dir}</span>
                <span className={`mr-1 ${resolved ? 'text-muted' : 'text-danger'}`}>
                    {normalizeEdgeKind(e.kind)}
                </span>
                {label.length > 35 ? '…' + label.slice(-33) : label}
            </ListItem>
        );
    };

    return (
        <div className="space-y-1 max-h-[30vh] overflow-auto">
            <p className="text-[10px] text-muted">Outbound ({outbound.length})</p>
            {outbound.slice(0, 50).map((e) => renderEdge(e, 'out'))}
            {outbound.length === 0 && <p className="text-[10px] text-muted px-2">none</p>}
            
            <p className="text-[10px] text-muted mt-2">Inbound ({inbound.length})</p>
            {inbound.slice(0, 50).map((e) => renderEdge(e, 'in'))}
            {inbound.length === 0 && <p className="text-[10px] text-muted px-2">none</p>}
        </div>
    );
}

function CallEdgeList({
    edges,
    selectedId,
    nodes,
    onSelect,
}: {
    edges: CallgraphEdge[];
    selectedId: string;
    nodes: Map<string, CallgraphNode>;
    onSelect: (id: string) => void;
}) {
    const outbound = edges.filter((e) => e.resolved && e.from === selectedId && e.to);
    const inbound = edges.filter((e) => e.resolved && e.to === selectedId);

    const selectedNode = nodes.get(selectedId);

    const openCallerAtCallsite = (callerId: string, line?: number | null) => {
        const caller = nodes.get(callerId);
        if (!caller?.file) {
            return;
        }
        vscode.postMessage({
            command: 'openFile',
            file: caller.file,
            line: typeof line === 'number' ? line : undefined,
        });
    };

    const openSelectedAtCallsite = (line?: number | null) => {
        if (!selectedNode?.file) {
            return;
        }
        vscode.postMessage({
            command: 'openFile',
            file: selectedNode.file,
            line: typeof line === 'number' ? line : undefined,
        });
    };

    return (
        <div className="space-y-1 max-h-[30vh] overflow-auto">
            <p className="text-[10px] text-muted">Callees ({outbound.length})</p>
            {outbound.slice(0, 40).map((e) => {
                const to = e.to as string;
                const target = nodes.get(to);
                const label = target ? target.display_name : e.to_raw;
                return (
                    <ListItem
                        key={`out-${e.from}-${e.to_raw}-${String(e.line ?? '')}`}
                        onClick={() => {
                            onSelect(to);
                            openSelectedAtCallsite(e.line ?? undefined);
                        }}
                    >
                        <span className="text-muted uppercase mr-1">out</span>
                        {label.length > 35 ? '…' + label.slice(-33) : label}
                    </ListItem>
                );
            })}
            {outbound.length === 0 && <p className="text-[10px] text-muted px-2">none</p>}

            <p className="text-[10px] text-muted mt-2">Callers ({inbound.length})</p>
            {inbound.slice(0, 40).map((e) => {
                const caller = nodes.get(e.from);
                const label = caller ? caller.display_name : e.from;
                return (
                    <ListItem
                        key={`in-${e.from}-${e.to_raw}-${String(e.line ?? '')}`}
                        onClick={() => {
                            onSelect(e.from);
                            openCallerAtCallsite(e.from, e.line ?? undefined);
                        }}
                    >
                        <span className="text-muted uppercase mr-1">in</span>
                        {label.length > 35 ? '…' + label.slice(-33) : label}
                    </ListItem>
                );
            })}
            {inbound.length === 0 && <p className="text-[10px] text-muted px-2">none</p>}
        </div>
    );
}

// Findings panel
function FindingsPanel({ findings }: { findings: GraphData['logic_findings'] }) {
    if (!findings || findings.length === 0) {
        return <p className="text-[10px] text-muted">No findings</p>;
    }

    return (
        <div className="space-y-0.5 max-h-[30vh] overflow-auto">
            {findings.slice(0, 30).map((f, i) => (
                <ListItem
                    key={i}
                    onClick={() => {
                        if (f.path) {
                            vscode.postMessage({
                                command: 'openFile',
                                file: f.path,
                                line: f.line,
                            });
                        }
                    }}
                >
                    <span className="text-muted mr-1">{f.category ?? 'issue'}</span>
                    {f.path}{f.line ? `:${f.line}` : ''} · {(f.message ?? '').slice(0, 30)}
                </ListItem>
            ))}
        </div>
    );
}

function CfgViewer({
    cfgs,
    selectedName,
    onSelect,
}: {
    cfgs: CfgItem[];
    selectedName: string;
    onSelect: (name: string) => void;
}) {
    const selected = cfgs.find((c) => c.name === selectedName) ?? cfgs[0];
    if (!selected) {
        return (
            <div className="flex items-center justify-center h-full text-[11px] text-muted">
                No CFG data
            </div>
        );
    }

    const hasMermaid = typeof selected.mermaid === 'string' && selected.mermaid.trim().length > 0;

    return (
        <div className="h-full flex flex-col">
            <div className="px-2 py-2 border-b border-border">
                <Row justify="between" gap="xs" className="flex-wrap">
                    <div className="min-w-[140px]">
                        <div className="text-[11px] font-semibold truncate">{selected.name}</div>
                        <div className="text-[10px] text-muted truncate">
                            {selected.language} · {selected.nodes} nodes · {selected.edges} edges · {selected.unreachable} unreachable
                        </div>
                    </div>
                    <Row gap="xs" className="flex-wrap">
                        {cfgs.length > 1 && (
                            <Select
                                value={selected.name}
                                onChange={(e) => onSelect((e.target as HTMLSelectElement).value)}
                            >
                                {cfgs.slice(0, 50).map((cfg) => (
                                    <option key={cfg.name} value={cfg.name}>
                                        {cfg.name}
                                    </option>
                                ))}
                            </Select>
                        )}
                        <Button
                            size="sm"
                            onClick={() =>
                                vscode.postMessage({
                                    command: 'openFile',
                                    file: selected.file,
                                    line: selected.start_line,
                                })
                            }
                        >
                            Open
                        </Button>
                        <Button
                            size="sm"
                            variant="ghost"
                            onClick={() =>
                                vscode.postMessage({
                                    command: 'copyText',
                                    text: JSON.stringify(selected, null, 2),
                                    label: 'CFG JSON',
                                })
                            }
                        >
                            Copy JSON
                        </Button>
                        <Button
                            size="sm"
                            variant="ghost"
                            disabled={!hasMermaid}
                            onClick={() =>
                                vscode.postMessage({
                                    command: 'copyText',
                                    text: selected.mermaid ?? '',
                                    label: 'Mermaid',
                                })
                            }
                        >
                            Copy Mermaid
                        </Button>
                    </Row>
                </Row>
            </div>
            <div className="p-2 overflow-auto flex-1">
                {hasMermaid ? (
                    <pre className="text-[10px] leading-snug whitespace-pre rounded border border-border bg-bg px-2 py-2 overflow-auto">
                        {selected.mermaid}
                    </pre>
                ) : (
                    <div className="text-[11px] text-muted">
                        Mermaid output not available for this function.
                    </div>
                )}
            </div>
        </div>
    );
}

// Main App
function App() {
    const initialUi = getWebviewState<UiState>({
        theme: getInitialTheme(),
        search: '',
        mode: 'blueprint',
    });

    const theme = useSignal<UiTheme>(initialUi.theme);
    const search = useSignal(initialUi.search);
    const mode = useSignal<'functions' | 'calls' | 'blueprint'>(initialUi.mode);

    const kindMod = useSignal(true);
    const kindUse = useSignal(true);
    const kindImport = useSignal(true);
    const cluster = useSignal('all');
    const focusCluster = useSignal(false);
    const indexRequested = useSignal(false);
    const didAutoIndex = useSignal(false);

    const indexData = useSignal<IndexData | null>(null);
    const blueprintData = useSignal<BlueprintReport | null>(null);
    const callgraphData = useSignal<CallgraphReport | null>(null);
    const graphData = useSignal<GraphData | null>(null);
    const selectedCfg = useSignal('');
    
    const selectedPath = useSignal('');
    const selectedFile = useSignal('');
    const selectedFn = useSignal('');
    const selectedCall = useSignal('');
    const error = useSignal<string | null>(null);

    // Apply theme
    useEffect(() => {
        applyTheme(theme.value);
        setWebviewState({ theme: theme.value, search: search.value, mode: mode.value });
    }, [theme.value, search.value, mode.value]);

    const allowedKinds = useMemo(() => {
        const set = new Set<string>();
        if (kindMod.value) set.add('mod');
        if (kindUse.value) set.add('use');
        if (kindImport.value) set.add('import');
        return set;
    }, [kindMod.value, kindUse.value, kindImport.value]);

    // Listen for messages
    useEffect(() => {
        const handler = (event: MessageEvent) => {
            const msg = event.data;
            if (!msg?.type) return;

            if (msg.type === 'indexData') {
                indexData.value = msg.data as IndexData;
                if (indexRequested.value) {
                    mode.value = 'functions';
                }
                indexRequested.value = false;
                error.value = null;
            }
            if (msg.type === 'blueprintData') {
                blueprintData.value = msg.data as BlueprintReport;
                selectedPath.value = msg.data?.nodes?.[0]?.path ?? '';
                mode.value = 'blueprint';
                indexRequested.value = false;
                error.value = null;

                const nodeCount = Array.isArray(msg.data?.nodes) ? msg.data.nodes.length : 0;
                if (!indexData.value && !didAutoIndex.value && nodeCount > 0 && nodeCount <= 500) {
                    didAutoIndex.value = true;
                    vscode.postMessage({ command: 'indexWorkspace' });
                }
            }
            if (msg.type === 'graphData') {
                graphData.value = msg.data as GraphData;
                selectedFile.value = msg.filePath ?? '';
                selectedFn.value = msg.functionName ?? '';
                mode.value = 'functions';
                error.value = null;

                const cfgs = Array.isArray((msg.data as any)?.cfgs) ? (msg.data as any).cfgs : [];
                if (cfgs.length > 0) {
                    const requested = typeof msg.functionName === 'string' ? msg.functionName : '';
                    const exact = requested ? cfgs.find((c: any) => c?.name === requested) : undefined;
                    const chosen = exact ?? cfgs[0];
                    selectedCfg.value = chosen?.name ?? '';
                } else {
                    selectedCfg.value = '';
                }
            }
            if (msg.type === 'callgraphData') {
                callgraphData.value = msg.data as CallgraphReport;
                selectedCall.value = (msg.data?.nodes?.[0]?.id as string) ?? '';
                mode.value = 'calls';
                error.value = null;
            }
            if (msg.type === 'error') {
                error.value = msg.message ?? 'Error';
            }
            if (msg.type === 'theme') {
                theme.value = coerceTheme(msg.value);
            }
        };
        window.addEventListener('message', handler);
        return () => window.removeEventListener('message', handler);
    }, []);

    const meta = useMemo(() => {
        if (!blueprintData.value) {
            return null;
        }
        return computeBlueprintMeta(blueprintData.value.nodes, blueprintData.value.edges, allowedKinds);
    }, [blueprintData.value, allowedKinds]);

    // Node path map for edge list
    const nodeMap = useMemo(() => {
        const map = new Map<string, BlueprintNode>();
        for (const n of blueprintData.value?.nodes ?? []) {
            map.set(n.path, n);
        }
        return map;
    }, [blueprintData.value]);

    const callNodeMap = useMemo(() => {
        const map = new Map<string, CallgraphNode>();
        for (const n of callgraphData.value?.nodes ?? []) {
            map.set(n.id, n);
        }
        return map;
    }, [callgraphData.value]);

    const callGraphElements = useMemo(() => {
        if (!callgraphData.value || !selectedCall.value) {
            return { nodes: [] as GraphNode[], edges: [] as GraphEdge[] };
        }
        const edges = (callgraphData.value.edges ?? []).filter((e) => e.resolved && (e.from === selectedCall.value || e.to === selectedCall.value));
        const nodeIds = new Set<string>();
        nodeIds.add(selectedCall.value);
        for (const e of edges) {
            nodeIds.add(e.from);
            if (e.to) nodeIds.add(e.to);
        }
        const nodes: GraphNode[] = (callgraphData.value.nodes ?? [])
            .filter((n) => nodeIds.has(n.id))
            .map((n) => ({
                id: n.id,
                label: n.display_name,
                language: n.language ?? 'code',
            }));
        const graphEdges: GraphEdge[] = edges
            .filter((e) => e.to)
            .map((e) => ({
                source: e.from,
                target: e.to as string,
                kind: e.kind,
                resolved: e.resolved,
            }));
        return { nodes, edges: graphEdges };
    }, [callgraphData.value, selectedCall.value]);

    const selectedClusterId = useMemo(() => {
        if (cluster.value === 'all') return null;
        const parsed = Number(cluster.value);
        return Number.isFinite(parsed) ? parsed : null;
    }, [cluster.value]);

    // Keep selectedPath inside the selected cluster (if any).
    useEffect(() => {
        if (!blueprintData.value || !meta || selectedClusterId === null) {
            return;
        }
        const inCluster = meta.clusters.find((c) => c.id === selectedClusterId);
        const first = inCluster?.nodes?.[0] ?? '';
        if (!first) {
            return;
        }
        if (!inCluster?.nodes?.includes(selectedPath.value)) {
            selectedPath.value = first;
        }
    }, [selectedClusterId, blueprintData.value, meta]);

    const activeClusterNodes = useMemo(() => {
        if (!meta || selectedClusterId === null) return null;
        const clusterInfo = meta.clusters.find((c) => c.id === selectedClusterId);
        if (!clusterInfo) return null;
        return new Set(clusterInfo.nodes);
    }, [meta, selectedClusterId]);

    // Convert blueprint to graph nodes/edges (cluster-aware).
    const graphElements = useMemo(() => {
        if (!blueprintData.value || !meta) return { nodes: [], edges: [] };

        const shouldFocus = focusCluster.value && activeClusterNodes;
        const visibleNodes = shouldFocus
            ? blueprintData.value.nodes.filter((n) => activeClusterNodes?.has(n.path))
            : blueprintData.value.nodes;

        const nodes: GraphNode[] = visibleNodes.map((n) => {
            const clusterId = meta.nodeToCluster.get(n.path) ?? 0;
            const dimmed = activeClusterNodes ? !activeClusterNodes.has(n.path) : false;
            return {
                id: n.path,
                label: n.path,
                language: n.language,
                lines: n.lines,
                isOrphan: meta.orphans.has(n.path),
                borderColor: clusterColor(clusterId),
                dimmed: shouldFocus ? false : dimmed,
            };
        });

        const edges = meta.resolvedEdges
            .filter((e) => {
                if (!activeClusterNodes) return true;
                const to = e.to!;
                if (!activeClusterNodes.has(e.from) || !activeClusterNodes.has(to)) {
                    return !shouldFocus;
                }
                return true;
            })
            .map((e) => {
                const to = e.to!;
                const inCluster =
                    activeClusterNodes ? activeClusterNodes.has(e.from) && activeClusterNodes.has(to) : true;
                return {
                    source: e.from,
                    target: to,
                    kind: normalizeEdgeKind(e.kind),
                    resolved: true,
                    dimmed: shouldFocus ? false : activeClusterNodes ? !inCluster : false,
                } satisfies GraphEdge;
            });

        return { nodes, edges };
    }, [blueprintData.value, meta, activeClusterNodes, focusCluster.value]);

    // Stats
    const stats = blueprintData.value?.stats;
    const clusterCount = meta?.clusters.length ?? 0;

    const selectedNode = nodeMap.get(selectedPath.value);
    const selectedDegrees = meta?.degrees.get(selectedPath.value) ?? { in: 0, out: 0, total: 0 };
    const selectedFns = useMemo(() => {
        if (!indexData.value || !selectedPath.value) return [];
        const items = Array.isArray(indexData.value.items) ? indexData.value.items : [];
        return items.filter((it) => it.file_display === selectedPath.value).slice(0, 80);
    }, [indexData.value, selectedPath.value]);

    const copyBlueprintJson = () => {
        if (!blueprintData.value) return;
        const text = JSON.stringify(blueprintData.value, null, 2);
        vscode.postMessage({ command: 'copyText', text, label: 'Flow blueprint JSON' });
    };

    const copyBlueprintContext = () => {
        if (!blueprintData.value || !meta) return;

        const clusterInfo =
            selectedClusterId === null ? null : meta.clusters.find((c) => c.id === selectedClusterId) ?? null;

        const outbound = blueprintData.value.edges
            .filter((e) => e.from === selectedPath.value)
            .filter((e) => allowedKinds.has(normalizeEdgeKind(e.kind)))
            .slice(0, 60)
            .map((e) => ({
                kind: normalizeEdgeKind(e.kind),
                to: e.to ?? null,
                to_raw: e.to_raw,
                line: e.line ?? null,
                resolved: Boolean(e.resolved && e.to),
            }));
        const inbound = blueprintData.value.edges
            .filter((e) => e.to === selectedPath.value)
            .filter((e) => allowedKinds.has(normalizeEdgeKind(e.kind)))
            .slice(0, 60)
            .map((e) => ({
                kind: normalizeEdgeKind(e.kind),
                from: e.from,
                to: e.to ?? null,
                to_raw: e.to_raw,
                line: e.line ?? null,
                resolved: Boolean(e.resolved && e.to),
            }));

        const ctx = {
            kind: 'toneguard.blueprint-context',
            version: 1,
            filters: {
                edge_kinds: Array.from(allowedKinds.values()).sort(),
                cluster: selectedClusterId === null ? 'all' : selectedClusterId,
                focus_cluster: Boolean(focusCluster.value && selectedClusterId !== null),
            },
            notes: {
                blueprint_edges: 'File-level dependencies from Rust mod/use + JS/TS/Py relative imports. Not a full call graph.',
                flow_index: indexData.value ? 'Loaded (functions/methods across repo).' : 'Not loaded.',
                cfg: 'Per-function CFG only (open a function to render).',
            },
            stats: {
                files: meta.clusters.reduce((sum, c) => sum + c.nodes.length, 0),
                edges: meta.resolvedEdges.length,
                clusters: meta.clusters.length,
                orphans: meta.orphans.size,
                sinks: meta.sinks.size,
            },
            hubs: meta.hubs.slice(0, 10),
            cluster: clusterInfo
                ? {
                      id: clusterInfo.id,
                      files: clusterInfo.nodes.length,
                      edges: clusterInfo.edges,
                      by_language: clusterInfo.byLanguage,
                  }
                : null,
            selected: selectedNode
                ? {
                      path: selectedNode.path,
                      language: selectedNode.language,
                      lines: selectedNode.lines,
                      size_bytes: selectedNode.size_bytes,
                      degree: selectedDegrees,
                      inbound,
                      outbound,
                      functions: selectedFns.map((f) => ({
                          name: f.display_name,
                          start_line: f.start_line,
                          kind: f.kind,
                      })),
                  }
                : null,
        };

        const text = JSON.stringify(ctx, null, 2);
        vscode.postMessage({ command: 'copyText', text, label: 'Blueprint context' });
    };

    return (
        <div className="min-h-full p-2 text-[11px]">
            {/* Header */}
            <Row justify="between" className="mb-2">
                <span className="text-[12px] font-semibold text-text">Flow Map</span>
                <Row gap="xs">
                    <Select
                        value={theme.value}
                        onChange={(e) => {
                            const val = (e.target as HTMLSelectElement).value as UiTheme;
                            theme.value = val;
                            vscode.postMessage({ command: 'setTheme', value: val });
                        }}
                    >
                        <option value="vscode">VS Code</option>
                        <option value="maple-light">Maple</option>
                    </Select>
                </Row>
            </Row>

            {/* Toolbar */}
            <Card padding="sm" className="mb-2">
                <Row gap="xs" className="flex-wrap">
                    <Button size="sm" onClick={() => vscode.postMessage({ command: 'selectFile' })}>
                        File
                    </Button>
                    <Button
                        size="sm"
                        onClick={() => {
                            indexRequested.value = true;
                            vscode.postMessage({ command: 'indexWorkspace' });
                        }}
                    >
                        Index
                    </Button>
                    <Button
                        size="sm"
                        onClick={() => vscode.postMessage({ command: 'callgraphWorkspace' })}
                    >
                        Calls
                    </Button>
                    <Button size="sm" variant="primary" onClick={() => vscode.postMessage({ command: 'blueprintWorkspace' })}>
                        Blueprint
                    </Button>
                    <Button size="sm" variant="ghost" disabled={!blueprintData.value} onClick={copyBlueprintContext}>
                        Copy LLM
                    </Button>
                    <Button size="sm" variant="ghost" disabled={!blueprintData.value} onClick={copyBlueprintJson}>
                        Copy JSON
                    </Button>
                    {stats && (
                        <span className="text-[10px] text-muted ml-2">
                            {stats.nodes} files · {stats.edges} edges · {stats.edges_resolved} resolved · {clusterCount} clusters
                        </span>
                    )}
                </Row>
                {blueprintData.value && meta && (
                    <Row gap="xs" className="flex-wrap mt-2">
                        <label className="flex items-center gap-1 text-[10px] text-muted">
                            <input
                                type="checkbox"
                                checked={kindMod.value}
                                onChange={(e) => (kindMod.value = (e.target as HTMLInputElement).checked)}
                            />
                            mod
                        </label>
                        <label className="flex items-center gap-1 text-[10px] text-muted">
                            <input
                                type="checkbox"
                                checked={kindUse.value}
                                onChange={(e) => (kindUse.value = (e.target as HTMLInputElement).checked)}
                            />
                            use
                        </label>
                        <label className="flex items-center gap-1 text-[10px] text-muted">
                            <input
                                type="checkbox"
                                checked={kindImport.value}
                                onChange={(e) => (kindImport.value = (e.target as HTMLInputElement).checked)}
                            />
                            import
                        </label>
                        {meta.clusters.length > 1 && (
                            <Select
                                value={cluster.value}
                                onChange={(e) => {
                                    cluster.value = (e.target as HTMLSelectElement).value;
                                    if ((e.target as HTMLSelectElement).value === 'all') {
                                        focusCluster.value = false;
                                    }
                                }}
                            >
                                <option value="all">All clusters</option>
                                {meta.clusters.slice(0, 30).map((c) => (
                                    <option key={c.id} value={String(c.id)}>
                                        c{c.id} · {c.nodes.length} files · {c.edges} edges
                                    </option>
                                ))}
                            </Select>
                        )}
                        {cluster.value !== 'all' && (
                            <label className="flex items-center gap-1 text-[10px] text-muted">
                                <input
                                    type="checkbox"
                                    checked={focusCluster.value}
                                    onChange={(e) => (focusCluster.value = (e.target as HTMLInputElement).checked)}
                                />
                                Focus
                            </label>
                        )}
                    </Row>
                )}
            </Card>

            {/* Main content */}
            <div className="grid grid-cols-1 lg:grid-cols-[280px_1fr] gap-2">
                {/* Sidebar */}
                <Stack gap="sm">
                    <Input
                        size="sm"
                        placeholder={mode.value === 'blueprint' ? 'Search files…' : 'Search functions…'}
                        value={search.value}
                        onInput={(e) => search.value = (e.target as HTMLInputElement).value}
                    />
                    <Card padding="sm">
                        {mode.value === 'blueprint' ? (
                            <FileList
                                items={blueprintData.value?.nodes ?? []}
                                search={search.value}
                                selectedId={selectedPath.value}
                                onSelect={(path) => selectedPath.value = path}
                            />
                        ) : mode.value === 'calls' ? (
                            <CallgraphList
                                items={callgraphData.value?.nodes ?? []}
                                search={search.value}
                                selectedId={selectedCall.value}
                                onSelect={(id) => selectedCall.value = id}
                            />
                        ) : (
                            <FunctionList
                                items={indexData.value?.items ?? []}
                                search={search.value}
                                selectedFile={selectedFile.value}
                                selectedFn={selectedFn.value}
                                onSelect={(file, fn) => {
                                    selectedFile.value = file;
                                    selectedFn.value = fn;
                                    vscode.postMessage({ command: 'loadFunction', file, functionName: fn });
                                }}
                            />
                        )}
                        {!blueprintData.value && !indexData.value && (
                            <p className="text-[10px] text-muted py-2">
                                Click Blueprint or Index to start
                            </p>
                        )}
                    </Card>
                </Stack>

                {/* Graph + Details */}
                <Stack gap="sm">
                    {/* Graph */}
                    <Card padding="none" className="h-[45vh] min-h-[300px]">
                        {mode.value === 'blueprint' && blueprintData.value && graphElements.nodes.length > 0 ? (
                            <Graph
                                nodes={graphElements.nodes}
                                edges={graphElements.edges}
                                selectedId={selectedPath.value}
                                onNodeClick={(id) => selectedPath.value = id}
                                onNodeDoubleClick={(id) => {
                                    const node = nodeMap.get(id);
                                    if (node?.abs_path) {
                                        vscode.postMessage({ command: 'openFile', file: node.abs_path });
                                    }
                                }}
                                theme={theme.value === 'maple-light' ? 'light' : 'dark'}
                                className="rounded"
                            />
                        ) : mode.value === 'calls' && callgraphData.value && callGraphElements.nodes.length > 0 ? (
                            <Graph
                                nodes={callGraphElements.nodes}
                                edges={callGraphElements.edges}
                                selectedId={selectedCall.value}
                                onNodeClick={(id) => selectedCall.value = id}
                                onNodeDoubleClick={(id) => {
                                    const node = callNodeMap.get(id);
                                    if (node?.file) {
                                        vscode.postMessage({ command: 'openFile', file: node.file, line: node.start_line });
                                    }
                                }}
                                theme={theme.value === 'maple-light' ? 'light' : 'dark'}
                                className="rounded"
                            />
                        ) : mode.value === 'functions' && graphData.value && (graphData.value.cfgs?.length ?? 0) > 0 ? (
                            <CfgViewer
                                cfgs={graphData.value.cfgs}
                                selectedName={selectedCfg.value}
                                onSelect={(name) => (selectedCfg.value = name)}
                            />
                        ) : error.value ? (
                            <div className="flex items-center justify-center h-full text-[11px] text-danger">
                                {error.value}
                            </div>
                        ) : (
                            <div className="flex items-center justify-center h-full text-[11px] text-muted">
                                {mode.value === 'calls'
                                    ? 'Run Calls to visualize a function neighborhood'
                                    : mode.value === 'functions'
                                        ? 'Index the workspace, then select a function to load its CFG'
                                        : 'Run Blueprint to visualize file dependencies'}
                            </div>
                        )}
                    </Card>

                    {/* Details Panel */}
                    <Card padding="sm">
                        <Row justify="between" className="mb-1">
                            <span className="text-[11px] font-semibold">
                                {mode.value === 'blueprint'
                                    ? 'Edges'
                                    : mode.value === 'calls'
                                        ? 'Calls'
                                        : 'Findings'}
                            </span>
                            <Badge variant="default">
                                {mode.value === 'blueprint'
                                    ? (blueprintData.value?.edges?.length ?? 0)
                                    : mode.value === 'calls'
                                        ? (callgraphData.value?.edges?.length ?? 0)
                                        : (graphData.value?.logic_findings?.length ?? 0)}
                            </Badge>
                        </Row>
                        {mode.value === 'blueprint' && blueprintData.value ? (
                            <EdgeList
                                edges={blueprintData.value.edges}
                                selectedPath={selectedPath.value}
                                nodes={nodeMap}
                                allowedKinds={allowedKinds}
                            />
                        ) : mode.value === 'calls' && callgraphData.value ? (
                            <CallEdgeList
                                edges={callgraphData.value.edges}
                                selectedId={selectedCall.value}
                                nodes={callNodeMap}
                                onSelect={(id) => selectedCall.value = id}
                            />
                        ) : (
                            <FindingsPanel findings={graphData.value?.logic_findings} />
                        )}
                    </Card>

                    {/* Selected file */}
                    {mode.value === 'blueprint' && selectedNode && meta && (
                        <Card padding="sm">
                            <Row justify="between" className="mb-1">
                                <span className="text-[11px] font-semibold">File</span>
                                <Badge variant="default">{selectedNode.language}</Badge>
                            </Row>
                            <div className="space-y-0.5 text-[10px] text-muted">
                                <div className="truncate">{selectedNode.path}</div>
                                <div>
                                    {selectedNode.lines} lines · {(selectedNode.size_bytes / 1024).toFixed(1)} KB · in{' '}
                                    {selectedDegrees.in} · out {selectedDegrees.out}
                                </div>
                            </div>
                            <Row gap="xs" className="mt-2 flex-wrap">
                                <Button
                                    size="sm"
                                    onClick={() =>
                                        vscode.postMessage({ command: 'openFile', file: selectedNode.abs_path })
                                    }
                                >
                                    Open
                                </Button>
                                <Button
                                    size="sm"
                                    variant="ghost"
                                    onClick={() =>
                                        vscode.postMessage({ command: 'copyText', text: selectedNode.path, label: 'File path' })
                                    }
                                >
                                    Copy path
                                </Button>
                                {!indexData.value && (
                                    <Button
                                        size="sm"
                                        variant="ghost"
                                        onClick={() => {
                                            indexRequested.value = false;
                                            vscode.postMessage({ command: 'indexWorkspace' });
                                        }}
                                    >
                                        Index functions
                                    </Button>
                                )}
                            </Row>
                            {indexData.value && (
                                <div className="mt-2">
                                    <Row justify="between" className="mb-1">
                                        <span className="text-[10px] text-muted">Functions</span>
                                        <Badge variant="default">{selectedFns.length}</Badge>
                                    </Row>
                                    <div className="space-y-0.5 max-h-[15vh] overflow-auto">
                                        {selectedFns.slice(0, 30).map((f, i) => (
                                            <ListItem
                                                key={`${f.file}-${f.target_name}-${i}`}
                                                onClick={() => {
                                                    selectedFile.value = f.file;
                                                    selectedFn.value = f.target_name;
                                                    vscode.postMessage({
                                                        command: 'loadFunction',
                                                        file: f.file,
                                                        functionName: f.target_name,
                                                    });
                                                }}
                                            >
                                                <span className="text-muted mr-1">{f.kind}</span>
                                                {f.display_name} · {f.start_line}
                                            </ListItem>
                                        ))}
                                        {selectedFns.length === 0 && (
                                            <p className="text-[10px] text-muted px-2 py-1">none</p>
                                        )}
                                    </div>
                                </div>
                            )}
                        </Card>
                    )}

                    {/* Orphan indicator */}
                    {mode.value === 'blueprint' && graphElements.nodes.filter(n => n.isOrphan).length > 0 && (
                        <Card padding="sm">
                            <Row justify="between" className="mb-1">
                                <span className="text-[11px] font-semibold text-danger">Orphans</span>
                                <Badge variant="danger">
                                    {graphElements.nodes.filter(n => n.isOrphan).length}
                                </Badge>
                            </Row>
                            <p className="text-[10px] text-muted">
                                Files with no inbound edges (potential dead code or entry points)
                            </p>
                            <div className="mt-1 space-y-0.5 max-h-[15vh] overflow-auto">
                                {graphElements.nodes.filter(n => n.isOrphan && !n.dimmed).slice(0, 20).map((n) => (
                                    <ListItem
                                        key={n.id}
                                        onClick={() => selectedPath.value = n.id}
                                    >
                                        <span className="text-muted mr-1">{n.language}</span>
                                        {n.label.length > 40 ? '…' + n.label.slice(-38) : n.label}
                                    </ListItem>
                                ))}
                            </div>
                        </Card>
                    )}

                    {/* Hubs */}
                    {mode.value === 'blueprint' && meta && meta.hubs.length > 0 && (
                        <Card padding="sm">
                            <Row justify="between" className="mb-1">
                                <span className="text-[11px] font-semibold">Hubs</span>
                                <Badge variant="default">{meta.hubs.length}</Badge>
                            </Row>
                            <p className="text-[10px] text-muted">High-degree files (good entry points for review)</p>
                            <div className="mt-1 space-y-0.5 max-h-[15vh] overflow-auto">
                                {meta.hubs.slice(0, 12).map((h) => (
                                    <ListItem key={h.path} onClick={() => selectedPath.value = h.path}>
                                        <span className="text-muted mr-1">{h.total}</span>
                                        {h.path.length > 40 ? '…' + h.path.slice(-38) : h.path}
                                    </ListItem>
                                ))}
                            </div>
                        </Card>
                    )}
                </Stack>
            </div>

            <div className="mt-3 pt-2 border-t border-border text-[10px] text-muted">
                <button
                    className="text-accent hover:text-text"
                    onClick={() => vscode.postMessage({ command: 'openGithubFeedback' })}
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
