import { useEffect, useRef } from 'preact/hooks';
import cytoscape, { Core, NodeSingular } from 'cytoscape';

export type GraphNode = {
    id: string;
    label: string;
    language?: string;
    lines?: number;
    isOrphan?: boolean;
    borderColor?: string;
    dimmed?: boolean;
};

export type GraphEdge = {
    source: string;
    target: string;
    kind?: string;
    resolved?: boolean;
    dimmed?: boolean;
};

interface GraphProps {
    nodes: GraphNode[];
    edges: GraphEdge[];
    selectedId?: string;
    onNodeClick?: (id: string) => void;
    onNodeDoubleClick?: (id: string) => void;
    className?: string;
    theme?: 'light' | 'dark';
}

const LANGUAGE_COLORS: Record<string, string> = {
    rust: '#dea584',
    typescript: '#3178c6',
    javascript: '#f7df1e',
    python: '#3776ab',
    go: '#00add8',
    java: '#b07219',
    c: '#555555',
    cpp: '#f34b7d',
    ruby: '#cc342d',
    default: '#6b7280',
};

function getNodeColor(language?: string): string {
    if (!language) return LANGUAGE_COLORS.default;
    const lower = language.toLowerCase();
    return LANGUAGE_COLORS[lower] ?? LANGUAGE_COLORS.default;
}

export function Graph({
    nodes,
    edges,
    selectedId,
    onNodeClick,
    onNodeDoubleClick,
    className = '',
    theme = 'light',
}: GraphProps) {
    const containerRef = useRef<HTMLDivElement>(null);
    const cyRef = useRef<Core | null>(null);

    useEffect(() => {
        if (!containerRef.current) return;

        const defaultBorder =
            theme === 'light' ? 'rgba(0,0,0,0.1)' : 'rgba(255,255,255,0.1)';

        const cy = cytoscape({
            container: containerRef.current,
            elements: [
                ...nodes.map((n) => ({
                    data: {
                        id: n.id,
                        label: n.label.length > 30 ? n.label.slice(-30) : n.label,
                        fullLabel: n.label,
                        language: n.language,
                        lines: n.lines,
                        isOrphan: n.isOrphan,
                        color: getNodeColor(n.language),
                        borderColor: n.borderColor ?? defaultBorder,
                        dimmed: n.dimmed,
                    },
                })),
                ...edges.filter((e) => e.resolved !== false).map((e, i) => ({
                    data: {
                        id: `e${i}`,
                        source: e.source,
                        target: e.target,
                        kind: e.kind,
                        dimmed: e.dimmed,
                    },
                })),
            ],
            style: [
                {
                    selector: 'node',
                    style: {
                        'label': 'data(label)',
                        'font-size': '10px',
                        'text-valign': 'center',
                        'text-halign': 'center',
                        'background-color': 'data(color)',
                        'color': theme === 'light' ? '#1a1a1a' : '#e5e5e5',
                        'text-outline-width': 1,
                        'text-outline-color': theme === 'light' ? '#ffffff' : '#1a1a1a',
                        'width': 'label',
                        'height': 24,
                        'padding': '8px',
                        'shape': 'round-rectangle',
                        'border-width': 1,
                        'border-color': 'data(borderColor)',
                    },
                },
                {
                    selector: 'node[?isOrphan]',
                    style: {
                        'border-width': 2,
                        'border-color': '#ef4444',
                        'border-style': 'dashed',
                    },
                },
                {
                    selector: 'node[?dimmed]',
                    style: {
                        opacity: 0.12,
                        'text-opacity': 0.1,
                    },
                },
                {
                    selector: 'node:selected',
                    style: {
                        'border-width': 2,
                        'border-color': '#0d7377',
                    },
                },
                {
                    selector: 'edge',
                    style: {
                        'width': 1,
                        'line-color': theme === 'light' ? 'rgba(0,0,0,0.2)' : 'rgba(255,255,255,0.2)',
                        'target-arrow-color': theme === 'light' ? 'rgba(0,0,0,0.3)' : 'rgba(255,255,255,0.3)',
                        'target-arrow-shape': 'triangle',
                        'curve-style': 'bezier',
                        'arrow-scale': 0.8,
                    },
                },
                {
                    selector: 'edge[?dimmed]',
                    style: {
                        opacity: 0.08,
                    },
                },
                {
                    selector: 'edge:selected',
                    style: {
                        'width': 2,
                        'line-color': '#0d7377',
                        'target-arrow-color': '#0d7377',
                    },
                },
            ],
            layout: {
                name: 'cose',
                animate: false,
                randomize: true,
                componentSpacing: 80,
                nodeRepulsion: () => 8000,
                idealEdgeLength: () => 100,
                edgeElasticity: () => 100,
                gravity: 0.25,
                numIter: 500,
            } as any,
            minZoom: 0.2,
            maxZoom: 3,
            wheelSensitivity: 0.3,
        });

        cy.on('tap', 'node', (evt) => {
            const node = evt.target as NodeSingular;
            onNodeClick?.(node.id());
        });

        cy.on('dbltap', 'node', (evt) => {
            const node = evt.target as NodeSingular;
            onNodeDoubleClick?.(node.id());
        });

        cyRef.current = cy;

        return () => {
            cy.destroy();
        };
    }, [nodes, edges, theme]);

    // Update selection
    useEffect(() => {
        if (!cyRef.current || !selectedId) return;
        cyRef.current.nodes().unselect();
        const node = cyRef.current.getElementById(selectedId);
        if (node.length > 0) {
            node.select();
            cyRef.current.animate({
                center: { eles: node },
                zoom: 1.5,
            }, { duration: 300 });
        }
    }, [selectedId]);

    return (
        <div
            ref={containerRef}
            className={`w-full h-full min-h-[300px] ${theme === 'light' ? 'bg-bg' : 'bg-surface-2'} ${className}`}
        />
    );
}

// Simplified mini graph for smaller displays
export function MiniGraph({
    nodes,
    edges,
    className = '',
}: {
    nodes: GraphNode[];
    edges: GraphEdge[];
    className?: string;
}) {
    const containerRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!containerRef.current) return;

        const cy = cytoscape({
            container: containerRef.current,
            elements: [
                ...nodes.slice(0, 50).map((n) => ({
                    data: { id: n.id, color: getNodeColor(n.language) },
                })),
                ...edges.filter(e => e.resolved !== false).slice(0, 100).map((e, i) => ({
                    data: { id: `e${i}`, source: e.source, target: e.target },
                })),
            ],
            style: [
                {
                    selector: 'node',
                    style: {
                        'width': 8,
                        'height': 8,
                        'background-color': 'data(color)',
                    },
                },
                {
                    selector: 'edge',
                    style: {
                        'width': 0.5,
                        'line-color': 'rgba(0,0,0,0.15)',
                    },
                },
            ],
            layout: { name: 'cose', animate: false, randomize: true } as any,
            userZoomingEnabled: false,
            userPanningEnabled: false,
            boxSelectionEnabled: false,
        });

        return () => cy.destroy();
    }, [nodes, edges]);

    return (
        <div ref={containerRef} className={`w-full h-24 ${className}`} />
    );
}
