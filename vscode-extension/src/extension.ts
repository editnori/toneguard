import { execFile } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

// Skill installation state key
const SKILL_PROMPT_DISMISSED_KEY = 'toneguard.skillPromptDismissed';

type AIEnvironment = 'cursor' | 'claude' | 'codex';
type SkillKind = 'writing' | 'logic-flow';

type DetectedEnvironments = {
    cursor: boolean;
    claude: boolean;
    codex: boolean;
};

type SidebarNode =
    | { kind: 'info'; label: string; description?: string }
    | { kind: 'action'; label: string; command: vscode.Command; description?: string }
    | { kind: 'category'; category: string; count: number }
    | { kind: 'finding'; finding: any };

type DashboardState = {
    platform: string;
    configPath: string;
    configSource: 'workspace' | 'bundled' | 'custom' | 'missing';
    cli: { mode: 'bundled' | 'PATH'; path: string };
    lsp: { mode: 'bundled' | 'PATH'; path: string };
    flowsCount: number | null;
    lastAudit: { when: string; findings: number } | null;
    lastProposal: { when: string } | null;
    detectedEnvs: DetectedEnvironments;
    skills: {
        writing: { cursor: boolean; claude: boolean; codex: boolean };
        'logic-flow': { cursor: boolean; claude: boolean; codex: boolean };
    };
    profileOptions: string[];
    currentProfile: string;
    settings: { strict: boolean; noRepoChecks: boolean };
    configEditable: boolean;
    // Config details for the new config panel
    enabledCategories: string[];
    ignoreGlobs: string[];
    enabledFileTypes: string[];
};

const SKILL_META: Record<
    SkillKind,
    { dirName: string; template: string; label: string; success: string }
> = {
    writing: {
        dirName: 'toneguard',
        template: 'toneguard-skill.md',
        label: 'ToneGuard Writing Style',
        success: 'The AI will now follow ToneGuard writing guidelines.',
    },
    'logic-flow': {
        dirName: 'toneguard-flow',
        template: 'toneguard-logic-flow-skill.md',
        label: 'ToneGuard Logic Flow Guardrails',
        success: 'The AI will now follow ToneGuard logic flow guardrails.',
    },
};

function getWorkspaceRoot(): string | undefined {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        return undefined;
    }
    return workspaceFolders[0].uri.fsPath;
}

function normalizeReportedPath(reported: string): string {
    return reported.replace(/^[.][/\\\\]/, '').replace(/\\/g, '/');
}

function relOrBasename(workspaceRoot: string, value: string): string {
    try {
        const cleaned = String(value);
        if (!cleaned) {
            return '<unset>';
        }
        const normalized = cleaned.replace(/\\/g, '/');
        if (path.isAbsolute(normalized)) {
            if (workspaceRoot && normalized.startsWith(workspaceRoot.replace(/\\/g, '/'))) {
                const rel = normalizeReportedPath(
                    path.relative(workspaceRoot, normalized)
                );
                return rel || path.basename(normalized);
            }
            return path.basename(normalized);
        }
        return normalized;
    } catch {
        return String(value);
    }
}

function fileMtimeLabel(filePath: string): string | undefined {
    try {
        const stat = fs.statSync(filePath);
        const when = stat.mtime;
        return when.toLocaleString();
    } catch {
        return undefined;
    }
}

function resolveConfigPath(
    workspaceRoot: string | undefined,
    configPath: string
): string | undefined {
    if (!configPath) {
        return undefined;
    }
    if (path.isAbsolute(configPath)) {
        return configPath;
    }
    if (workspaceRoot) {
        return path.join(workspaceRoot, configPath);
    }
    return configPath;
}

function isPathInside(baseDir: string, targetPath: string): boolean {
    const rel = path.relative(baseDir, targetPath);
    return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel));
}

class ToneGuardSidebarProvider implements vscode.TreeDataProvider<SidebarNode> {
    private readonly onDidChangeEmitter = new vscode.EventEmitter<
        SidebarNode | undefined
    >();
    readonly onDidChangeTreeData = this.onDidChangeEmitter.event;

    refresh(): void {
        this.onDidChangeEmitter.fire(undefined);
    }

    getTreeItem(element: SidebarNode): vscode.TreeItem {
        if (element.kind === 'info') {
            const item = new vscode.TreeItem(
                element.label,
                vscode.TreeItemCollapsibleState.None
            );
            item.description = element.description;
            item.contextValue = 'toneguard.info';
            return item;
        }

        if (element.kind === 'action') {
            const item = new vscode.TreeItem(
                element.label,
                vscode.TreeItemCollapsibleState.None
            );
            item.command = element.command;
            item.description = element.description;
            item.contextValue = 'toneguard.action';
            return item;
        }

        if (element.kind === 'category') {
            const item = new vscode.TreeItem(
                `${element.category} (${element.count})`,
                element.count > 0
                    ? vscode.TreeItemCollapsibleState.Collapsed
                    : vscode.TreeItemCollapsibleState.None
            );
            item.contextValue = 'toneguard.category';
            return item;
        }

        // Finding node
        const finding = element.finding as any;
        const filePath = typeof finding?.path === 'string' ? finding.path : '<unknown>';
        const line = typeof finding?.line === 'number' ? finding.line : undefined;
        const message =
            typeof finding?.message === 'string' ? finding.message : 'Finding';
        const severity =
            typeof finding?.severity === 'string' ? finding.severity : 'info';

        const loc = line ? `:${line}` : '';
        const label = message.length > 80 ? `${message.slice(0, 77)}...` : message;

        const item = new vscode.TreeItem(
            label,
            vscode.TreeItemCollapsibleState.None
        );
        item.description = `${normalizeReportedPath(filePath)}${loc}`;
        item.tooltip = `[${severity}] ${message}\n\nClick to open file`;
        item.command = {
            command: 'dwg.openFindingLocation',
            title: 'Open Finding',
            arguments: [filePath, line],
        };
        item.contextValue = 'toneguard.finding';
        return item;
    }

    async getChildren(element?: SidebarNode): Promise<SidebarNode[]> {
        const root = getWorkspaceRoot();
        if (!root) {
            return [
                {
                    kind: 'info',
                    label: 'Open a folder to enable ToneGuard',
                    description: '',
                },
            ];
        }

        // Root level: show findings grouped by category
        if (!element) {
            const report = this.readAuditReport(root);
            if (!report) {
                return [
                    {
                        kind: 'action',
                        label: 'Run ToneGuard to see findings',
                        command: { command: 'dwg.runRecommended', title: 'Run' },
                        description: 'Click to run audit',
                    },
                ];
            }

            const findings = Array.isArray(report?.audit?.findings)
                ? report.audit.findings
                : [];
            const counts = new Map<string, number>();
            for (const finding of findings) {
                const category =
                    typeof finding?.category === 'string'
                        ? finding.category
                        : 'unknown';
                counts.set(category, (counts.get(category) ?? 0) + 1);
            }

            const categories = Array.from(counts.entries()).sort((a, b) => {
                if (b[1] !== a[1]) {
                    return b[1] - a[1];
                }
                return a[0].localeCompare(b[0]);
            });

            if (categories.length === 0) {
                return [
                    {
                        kind: 'info',
                        label: 'No findings - code is clean!',
                        description: '',
                    },
                ];
            }

            return categories.map(([category, count]) => ({
                kind: 'category',
                category,
                count,
            }));
        }

        // Category level: show individual findings
        if (element.kind === 'category') {
            const report = this.readAuditReport(root);
            const findings = Array.isArray(report?.audit?.findings)
                ? report.audit.findings
                : [];
            const categoryFindings = findings.filter(
                (f: any) => typeof f?.category === 'string' && f.category === element.category
            );
            return categoryFindings.map((finding: any) => ({
                kind: 'finding',
                finding,
            }));
        }

        return [];
    }

    private readAuditReport(workspaceRoot: string): any | undefined {
        try {
            const reportPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
            if (!fs.existsSync(reportPath)) {
                return undefined;
            }
            const text = fs.readFileSync(reportPath, 'utf8');
            return JSON.parse(text);
        } catch {
            return undefined;
        }
    }
}

class ToneGuardDashboardProvider implements vscode.WebviewViewProvider {
    private view?: vscode.WebviewView;

    constructor(
        private readonly context: vscode.ExtensionContext,
        private readonly outputChannel: vscode.OutputChannel,
        private readonly serverCommand: string,
        private readonly getRuntimeConfig: () => {
            cliCommand: string;
            configPath: string;
            profile: string;
        }
    ) {}

    async resolveWebviewView(view: vscode.WebviewView): Promise<void> {
        this.view = view;
        view.webview.options = {
            enableScripts: true,
            localResourceRoots: [this.context.extensionUri],
        };

        const state = await this.collectState();
        view.webview.html = this.getHtmlForWebview(view.webview, state);

        view.webview.onDidReceiveMessage(async (message) => {
            await this.handleMessage(message);
        });
    }

    async refresh(): Promise<void> {
        if (!this.view) {
            return;
        }
        const state = await this.collectState();
        void this.view.webview.postMessage({ type: 'state', state });
    }

    private async handleMessage(message: any): Promise<void> {
        if (!message || typeof message.type !== 'string') {
            return;
        }

        switch (message.type) {
            case 'runRecommended':
                void vscode.commands.executeCommand('dwg.runRecommended');
                return;
            case 'runAudit':
                void vscode.commands.executeCommand('dwg.flowAudit');
                return;
            case 'runProposal':
                void vscode.commands.executeCommand('dwg.flowPropose');
                return;
            case 'newFlow':
                void vscode.commands.executeCommand('dwg.flowNew');
                return;
            case 'openDocs':
                void vscode.commands.executeCommand('dwg.openDocs');
                return;
            case 'openConfig': {
                const config = this.getRuntimeConfig();
                await openConfigFile(this.context, config.configPath);
        return;
            }
            case 'addIgnore': {
                const raw = typeof message.value === 'string' ? message.value : '';
                const cleaned = raw.trim();
                if (!cleaned) {
                    void vscode.window.showErrorMessage(
                        'ToneGuard: enter a glob to ignore.'
                    );
                    return;
                }
                const config = this.getRuntimeConfig();
                const result = await addIgnoreGlobToConfig(
                    this.context,
                    config.configPath,
                    cleaned,
                    this.outputChannel
                );
                if (result) {
                    await this.refresh();
                }
                return;
            }
            case 'setProfile': {
                const value = typeof message.value === 'string' ? message.value : '';
                const config = vscode.workspace.getConfiguration('dwg');
                await config.update(
                    'profile',
                    value,
                    vscode.ConfigurationTarget.Workspace
                );
                await this.refresh();
                return;
            }
            case 'clearProfile': {
                const config = vscode.workspace.getConfiguration('dwg');
                await config.update(
                    'profile',
                    '',
                    vscode.ConfigurationTarget.Workspace
                );
                await this.refresh();
                return;
            }
            case 'toggleSetting': {
                const key = typeof message.key === 'string' ? message.key : '';
                if (key !== 'strict' && key !== 'noRepoChecks') {
                    return;
                }
                const value = Boolean(message.value);
                const config = vscode.workspace.getConfiguration('dwg');
                await config.update(
                    key,
                    value,
                    vscode.ConfigurationTarget.Workspace
                );
                await this.refresh();
                return;
            }
            case 'installSkill': {
                const envRaw = typeof message.env === 'string' ? message.env : '';
                const kindRaw = typeof message.kind === 'string' ? message.kind : '';
                const kind = kindRaw === 'logic-flow' ? 'logic-flow' : 'writing';
                if (envRaw === 'all') {
                    await installSkill(this.context, 'cursor', this.outputChannel, kind);
                    await installSkill(this.context, 'claude', this.outputChannel, kind);
                    await installSkill(this.context, 'codex', this.outputChannel, kind);
                } else if (envRaw === 'cursor' || envRaw === 'claude' || envRaw === 'codex') {
                    await installSkill(
                        this.context,
                        envRaw,
                        this.outputChannel,
                        kind
                    );
                }
                await this.refresh();
                return;
            }
            case 'toggleCategory': {
                const category = typeof message.category === 'string' ? message.category : '';
                const enabled = Boolean(message.enabled);
                const config = this.getRuntimeConfig();
                await toggleCategoryInConfig(
                    this.context,
                    config.configPath,
                    category,
                    enabled,
                    this.outputChannel
                );
                await this.refresh();
                return;
            }
            case 'removeIgnore': {
                const glob = typeof message.glob === 'string' ? message.glob : '';
                const config = this.getRuntimeConfig();
                await removeIgnoreGlobFromConfig(
                    this.context,
                    config.configPath,
                    glob,
                    this.outputChannel
                );
                await this.refresh();
                return;
            }
            case 'toggleFileType': {
                const fileType = typeof message.fileType === 'string' ? message.fileType : '';
                const enabled = Boolean(message.enabled);
                const config = this.getRuntimeConfig();
                await toggleFileTypeInConfig(
                    this.context,
                    config.configPath,
                    fileType,
                    enabled,
                    this.outputChannel
                );
                await this.refresh();
                return;
            }
            case 'fixWithCursor': {
                const findings = message.findings;
                if (Array.isArray(findings) && findings.length > 0) {
                    await fixWithAI(this.context, findings, 'cursor', this.outputChannel);
                }
                return;
            }
            case 'fixWithClaude': {
                const findings = message.findings;
                if (Array.isArray(findings) && findings.length > 0) {
                    await fixWithAI(this.context, findings, 'claude', this.outputChannel);
                }
                return;
            }
            case 'fixWithCodex': {
                const findings = message.findings;
                if (Array.isArray(findings) && findings.length > 0) {
                    await fixWithAI(this.context, findings, 'codex', this.outputChannel);
                }
                return;
            }
            default:
                return;
        }
    }

    private async collectState(): Promise<DashboardState> {
        const workspaceRoot = getWorkspaceRoot();
        const runtime = this.getRuntimeConfig();
        const platform = getPlatformDir();
        const bundledCli = getBundledCliPath(this.context);
        const bundledServer = getBundledServerPath(this.context);

        const absConfigPath = resolveConfigPath(workspaceRoot, runtime.configPath);
        const configExists = absConfigPath ? fs.existsSync(absConfigPath) : false;

        let configSource: DashboardState['configSource'] = 'missing';
        if (configExists && absConfigPath) {
            if (absConfigPath.startsWith(this.context.extensionPath)) {
                configSource = 'bundled';
            } else if (workspaceRoot && isPathInside(workspaceRoot, absConfigPath)) {
                configSource = 'workspace';
            } else {
                configSource = 'custom';
            }
        }

        const configShown =
            workspaceRoot && absConfigPath
                ? relOrBasename(workspaceRoot, absConfigPath)
                : runtime.configPath || '<unset>';

        const flowsDir = workspaceRoot ? path.join(workspaceRoot, 'flows') : '';
        let flowsCount: number | null = null;
        if (flowsDir && fs.existsSync(flowsDir)) {
            try {
                flowsCount = fs
                    .readdirSync(flowsDir)
                    .filter((name) => name.endsWith('.md') || name.endsWith('.yaml') || name.endsWith('.yml'))
                    .length;
            } catch {
                flowsCount = null;
            }
        }

        let lastAudit: DashboardState['lastAudit'] = null;
        if (workspaceRoot) {
            const auditPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
            if (fs.existsSync(auditPath)) {
                try {
                    const text = fs.readFileSync(auditPath, 'utf8');
                    const report = JSON.parse(text);
                    const findings = Array.isArray(report?.audit?.findings)
                        ? report.audit.findings
                        : [];
                    const when = fileMtimeLabel(auditPath);
                    lastAudit = {
                        when: when ?? 'reports/flow-audit.json',
                        findings: findings.length,
                    };
                } catch {
                    lastAudit = null;
                }
            }
        }

        let lastProposal: DashboardState['lastProposal'] = null;
        if (workspaceRoot) {
            const proposalPath = path.join(workspaceRoot, 'reports', 'flow-proposal.md');
            if (fs.existsSync(proposalPath)) {
                const when = fileMtimeLabel(proposalPath);
                lastProposal = { when: when ?? 'reports/flow-proposal.md' };
            }
        }

        const detectedEnvs = detectAIEnvironments();
        const profileOptions =
            absConfigPath && fs.existsSync(absConfigPath)
                ? extractProfilesFromConfig(absConfigPath)
                : [];

        const config = vscode.workspace.getConfiguration('dwg');
        const strict = config.get<boolean>('strict', false);
        const noRepoChecks = config.get<boolean>('noRepoChecks', false);

        const configEditable =
            configExists && workspaceRoot && absConfigPath
                ? isPathInside(workspaceRoot, absConfigPath)
                : false;

        // Extract config details for the config panel
        const disabledCategories = absConfigPath && configExists
            ? extractDisabledCategoriesFromConfig(absConfigPath)
            : [];
        const enabledCategories = ALL_CATEGORIES.filter(c => !disabledCategories.includes(c));
        const ignoreGlobs = absConfigPath && configExists
            ? extractIgnoreGlobsFromConfig(absConfigPath)
            : [];
        const enabledFileTypes = absConfigPath && configExists
            ? extractEnabledFileTypesFromConfig(absConfigPath)
            : ALL_FILE_TYPES;

        return {
            platform,
            configPath: configShown,
            configSource,
            cli: {
                mode:
                    bundledCli && runtime.cliCommand === bundledCli
                        ? 'bundled'
                        : 'PATH',
                path: relOrBasename(workspaceRoot ?? '', runtime.cliCommand),
            },
            lsp: {
                mode:
                    bundledServer && this.serverCommand === bundledServer
                        ? 'bundled'
                        : 'PATH',
                path: relOrBasename(workspaceRoot ?? '', this.serverCommand),
            },
            flowsCount,
            lastAudit,
            lastProposal,
            detectedEnvs,
            skills: {
                writing: {
                    cursor: isSkillInstalled('cursor', 'writing'),
                    claude: isSkillInstalled('claude', 'writing'),
                    codex: isSkillInstalled('codex', 'writing'),
                },
                'logic-flow': {
                    cursor: isSkillInstalled('cursor', 'logic-flow'),
                    claude: isSkillInstalled('claude', 'logic-flow'),
                    codex: isSkillInstalled('codex', 'logic-flow'),
                },
            },
            profileOptions,
            currentProfile: runtime.profile,
            settings: {
                strict,
                noRepoChecks,
            },
            configEditable,
            enabledCategories,
            ignoreGlobs,
            enabledFileTypes,
        };
    }

    private getHtmlForWebview(
        webview: vscode.Webview,
        state: DashboardState
    ): string {
        const nonce = getNonce();
        const stateJson = JSON.stringify(state).replace(/</g, '\\u003c');

        return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${webview.cspSource} https:; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <style>
        :root {
            --tg-bg: var(--vscode-sideBar-background);
            --tg-card: var(--vscode-editorWidget-background);
            --tg-border: var(--vscode-widget-border);
            --tg-text: var(--vscode-foreground);
            --tg-muted: var(--vscode-descriptionForeground);
            --tg-accent: var(--vscode-charts-blue);
            --tg-accent-2: var(--vscode-charts-green);
            --tg-warn: var(--vscode-charts-orange);
        }

        body {
            margin: 0;
            padding: 0;
            font-family: var(--vscode-font-family);
            color: var(--tg-text);
            background: var(--tg-bg);
        }

        .container {
            padding: 14px;
            display: grid;
            gap: 14px;
        }

        .section-title {
            font-size: 11px;
            letter-spacing: 0.12em;
            text-transform: uppercase;
            color: var(--tg-muted);
            margin: 0 0 8px;
        }

        .card {
            background: linear-gradient(135deg, rgba(255,255,255,0.02), rgba(0,0,0,0.05)), var(--tg-card);
            border: 1px solid var(--tg-border);
            border-radius: 12px;
            padding: 12px;
            display: grid;
            gap: 8px;
        }

        .card-row {
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 8px;
        }

        .label {
            font-size: 12px;
            color: var(--tg-muted);
        }

        .value {
            font-size: 12px;
        }

        .badge {
            padding: 2px 8px;
            border-radius: 999px;
            font-size: 11px;
            background: rgba(255,255,255,0.05);
            border: 1px solid var(--tg-border);
        }

        .badge.ok {
            border-color: var(--tg-accent-2);
            color: var(--tg-accent-2);
        }

        .badge.warn {
            border-color: var(--tg-warn);
            color: var(--tg-warn);
        }

        .actions {
            display: grid;
            gap: 8px;
        }

        button {
            border: 1px solid var(--tg-border);
            background: var(--vscode-button-background);
            color: var(--vscode-button-foreground);
            padding: 6px 10px;
            border-radius: 8px;
            font-size: 12px;
            cursor: pointer;
        }

        button.secondary {
            background: var(--vscode-button-secondaryBackground);
            color: var(--vscode-button-secondaryForeground);
        }

        button.ghost {
            background: transparent;
            color: var(--tg-text);
        }

        button:disabled {
            opacity: 0.6;
            cursor: default;
        }

        .grid-2 {
            display: grid;
            grid-template-columns: repeat(2, minmax(0, 1fr));
            gap: 8px;
        }

        .pill {
            padding: 4px 10px;
            border-radius: 999px;
            border: 1px solid var(--tg-border);
            font-size: 11px;
            cursor: pointer;
            background: transparent;
            color: var(--tg-text);
        }

        .pill.active {
            border-color: var(--tg-accent);
            color: var(--tg-accent);
        }

        .input-row {
            display: flex;
            gap: 6px;
            align-items: center;
        }

        input[type="text"] {
            flex: 1;
            border: 1px solid var(--tg-border);
            border-radius: 8px;
            padding: 6px 8px;
            background: var(--vscode-input-background);
            color: var(--vscode-input-foreground);
        }

        .toggle {
            display: inline-flex;
            align-items: center;
            gap: 6px;
            font-size: 12px;
        }

        .divider {
            height: 1px;
            background: var(--tg-border);
            margin: 4px 0;
        }

        .toggle-grid {
            display: grid;
            grid-template-columns: repeat(2, 1fr);
            gap: 6px;
        }

        .toggle-grid label {
            display: flex;
            align-items: center;
            gap: 6px;
            font-size: 12px;
            cursor: pointer;
        }

        .toggle-grid input[type="checkbox"] {
            margin: 0;
        }

        .tag-list {
            display: flex;
            flex-wrap: wrap;
            gap: 6px;
            margin-bottom: 8px;
        }

        .tag {
            display: inline-flex;
            align-items: center;
            gap: 4px;
            padding: 3px 8px;
            background: rgba(255,255,255,0.05);
            border: 1px solid var(--tg-border);
            border-radius: 999px;
            font-size: 11px;
        }

        .tag button {
            padding: 0 4px;
            border: none;
            background: transparent;
            color: var(--tg-muted);
            cursor: pointer;
            font-size: 12px;
        }

        .tag button:hover {
            color: var(--vscode-errorForeground);
        }

        .help {
            color: var(--tg-muted);
            cursor: help;
            font-size: 11px;
        }

        .fix-buttons {
            display: flex;
            gap: 6px;
            flex-wrap: wrap;
        }

        .fix-buttons button {
            flex: 1;
            min-width: 80px;
        }

        .empty-state {
            color: var(--tg-muted);
            font-size: 11px;
            font-style: italic;
        }
    </style>
</head>
<body>
    <div class="container">
        <div>
            <div class="section-title">Run</div>
            <div class="card actions">
                <button data-action="runRecommended">Run Recommended</button>
                <div class="grid-2">
                    <button class="secondary" data-action="runAudit">Flow Audit</button>
                    <button class="secondary" data-action="runProposal">Flow Proposal</button>
                </div>
                <button class="ghost" data-action="newFlow">New Flow Spec</button>
            </div>
        </div>

        <div>
            <div class="section-title">Status</div>
            <div class="card">
                <div class="card-row">
                    <span class="label">Platform</span>
                    <span class="value" id="platform"></span>
                </div>
                <div class="card-row">
                    <span class="label">Config</span>
                    <span class="value" id="configPath"></span>
                </div>
                <div class="card-row">
                    <span class="label">Config source</span>
                    <span class="badge" id="configSource"></span>
                </div>
                <div class="card-row">
                    <span class="label">CLI</span>
                    <span class="value" id="cliPath"></span>
                </div>
                <div class="card-row">
                    <span class="label">LSP</span>
                    <span class="value" id="lspPath"></span>
                </div>
                <div class="divider"></div>
                <div class="card-row">
                    <span class="label">Flows</span>
                    <span class="value" id="flowsCount"></span>
                </div>
                <div class="card-row">
                    <span class="label">Last audit</span>
                    <span class="value" id="lastAudit"></span>
                </div>
                <div class="card-row">
                    <span class="label">Last proposal</span>
                    <span class="value" id="lastProposal"></span>
                </div>
            </div>
        </div>

        <div>
            <div class="section-title">AI Skills</div>
            <div class="card">
                <div class="card-row">
                    <strong class="value">Writing Style</strong>
                    <button class="secondary" data-action="installSkill" data-kind="writing" data-env="all">Install All</button>
                </div>
                <div class="card-row" data-skill="writing" data-env="cursor">
                    <span class="label">Cursor</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="writing" data-env="cursor">Install</button>
                    </div>
                </div>
                <div class="card-row" data-skill="writing" data-env="claude">
                    <span class="label">Claude Code</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="writing" data-env="claude">Install</button>
                    </div>
                </div>
                <div class="card-row" data-skill="writing" data-env="codex">
                    <span class="label">Codex</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="writing" data-env="codex">Install</button>
                    </div>
                </div>
                <div class="divider"></div>
                <div class="card-row">
                    <strong class="value">Logic Flow Guardrails</strong>
                    <button class="secondary" data-action="installSkill" data-kind="logic-flow" data-env="all">Install All</button>
                </div>
                <div class="card-row" data-skill="logic-flow" data-env="cursor">
                    <span class="label">Cursor</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="logic-flow" data-env="cursor">Install</button>
                    </div>
                </div>
                <div class="card-row" data-skill="logic-flow" data-env="claude">
                    <span class="label">Claude Code</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="logic-flow" data-env="claude">Install</button>
                    </div>
                </div>
                <div class="card-row" data-skill="logic-flow" data-env="codex">
                    <span class="label">Codex</span>
                    <div class="input-row">
                        <span class="badge" data-skill-badge>Not installed</span>
                        <button data-action="installSkill" data-kind="logic-flow" data-env="codex">Install</button>
                    </div>
                </div>
            </div>
        </div>

        <div>
            <div class="section-title">Categories <span class="help" title="Toggle which checks to run">(?)</span></div>
            <div class="card">
                <div id="categoryToggles" class="toggle-grid"></div>
            </div>
        </div>

        <div>
            <div class="section-title">Ignore Patterns</div>
            <div class="card">
                <div id="ignoreGlobsList" class="tag-list"></div>
                <div class="input-row">
                    <input id="ignoreInput" type="text" placeholder="e.g. **/dist/**" />
                    <button class="secondary" data-action="addIgnore">Add</button>
                </div>
            </div>
        </div>

        <div>
            <div class="section-title">File Types</div>
            <div class="card">
                <div id="fileTypeToggles" class="toggle-grid"></div>
            </div>
        </div>

        <div>
            <div class="section-title">Profiles <span class="help" title="Profiles apply different rules to different file types">(?)</span></div>
            <div class="card">
                <div class="card-row">
                    <span class="label">Active profile</span>
                    <span class="value" id="currentProfile"></span>
                </div>
                <div class="input-row">
                    <button class="ghost" data-action="clearProfile">Clear</button>
                </div>
                <div id="profileOptions" class="input-row" style="flex-wrap: wrap;"></div>
            </div>
        </div>

        <div>
            <div class="section-title">Config</div>
            <div class="card">
                <button class="secondary" data-action="openConfig">Open Config File</button>
            </div>
        </div>

        <div>
            <div class="section-title">Quick Settings</div>
            <div class="card">
                <label class="toggle">
                    <input id="strictToggle" type="checkbox" />
                    Strict mode (surface warnings as errors)
                </label>
                <label class="toggle">
                    <input id="repoToggle" type="checkbox" />
                    Skip repo-wide checks
                </label>
            </div>
        </div>
    </div>

    <script nonce="${nonce}">
        const vscode = acquireVsCodeApi();
        const state = ${stateJson};

        function setText(id, value) {
            const el = document.getElementById(id);
            if (el) {
                el.textContent = value;
            }
        }

        function render(state) {
            setText('platform', state.platform || 'unknown');
            setText('configPath', state.configPath || 'unknown');
            setText('configSource', state.configSource || 'unknown');
            const configBadge = document.getElementById('configSource');
            if (configBadge) {
                configBadge.classList.remove('ok', 'warn');
                if (state.configSource === 'workspace') {
                    configBadge.classList.add('ok');
                } else if (state.configSource === 'bundled') {
                    configBadge.classList.add('warn');
                }
            }

            const cliLabel = state.cli.mode + ' · ' + (state.cli.path || 'dwg');
            const lspLabel = state.lsp.mode + ' · ' + (state.lsp.path || 'dwg-lsp');
            setText('cliPath', cliLabel);
            setText('lspPath', lspLabel);

            setText(
                'flowsCount',
                state.flowsCount === null ? 'unknown' : String(state.flowsCount)
            );

            if (state.lastAudit) {
                setText(
                    'lastAudit',
                    state.lastAudit.findings + ' findings · ' + state.lastAudit.when
                );
            } else {
                setText('lastAudit', 'none');
            }

            if (state.lastProposal) {
                setText('lastProposal', state.lastProposal.when);
            } else {
                setText('lastProposal', 'none');
            }

            setText(
                'currentProfile',
                state.currentProfile ? state.currentProfile : 'auto'
            );

            // Render category toggles
            const categoryWrap = document.getElementById('categoryToggles');
            if (categoryWrap) {
                categoryWrap.innerHTML = '';
                const allCats = ['buzzword', 'puffery', 'confidence', 'filler', 'transition', 'hedge', 'emoji', 'cliche', 'passive', 'uniformity', 'repetition'];
                const enabled = Array.isArray(state.enabledCategories) ? state.enabledCategories : allCats;
                allCats.forEach((cat) => {
                    const label = document.createElement('label');
                    const checkbox = document.createElement('input');
                    checkbox.type = 'checkbox';
                    checkbox.checked = enabled.includes(cat);
                    checkbox.dataset.category = cat;
                    label.appendChild(checkbox);
                    label.appendChild(document.createTextNode(cat));
                    categoryWrap.appendChild(label);
                });
            }

            // Render ignore globs
            const ignoreWrap = document.getElementById('ignoreGlobsList');
            if (ignoreWrap) {
                ignoreWrap.innerHTML = '';
                const globs = Array.isArray(state.ignoreGlobs) ? state.ignoreGlobs : [];
                if (globs.length === 0) {
                    const empty = document.createElement('span');
                    empty.className = 'empty-state';
                    empty.textContent = 'No ignore patterns';
                    ignoreWrap.appendChild(empty);
                } else {
                    globs.forEach((glob) => {
                        const tag = document.createElement('span');
                        tag.className = 'tag';
                        tag.textContent = glob + ' ';
                        const btn = document.createElement('button');
                        btn.textContent = '×';
                        btn.dataset.action = 'removeIgnore';
                        btn.dataset.glob = glob;
                        tag.appendChild(btn);
                        ignoreWrap.appendChild(tag);
                    });
                }
            }

            // Render file type toggles
            const fileTypeWrap = document.getElementById('fileTypeToggles');
            if (fileTypeWrap) {
                fileTypeWrap.innerHTML = '';
                const allTypes = ['.md', '.txt', '.rst', '.ts', '.tsx', '.js', '.jsx', '.py', '.rs'];
                const enabled = Array.isArray(state.enabledFileTypes) ? state.enabledFileTypes : allTypes;
                allTypes.forEach((ft) => {
                    const label = document.createElement('label');
                    const checkbox = document.createElement('input');
                    checkbox.type = 'checkbox';
                    checkbox.checked = enabled.includes(ft);
                    checkbox.dataset.filetype = ft;
                    label.appendChild(checkbox);
                    label.appendChild(document.createTextNode(ft));
                    fileTypeWrap.appendChild(label);
                });
            }

            const profileWrap = document.getElementById('profileOptions');
            if (profileWrap) {
                profileWrap.innerHTML = '';
                const options = Array.isArray(state.profileOptions)
                    ? state.profileOptions
                    : [];
                if (options.length === 0) {
                    const empty = document.createElement('span');
                    empty.className = 'label';
                    empty.textContent = 'No profiles found';
                    profileWrap.appendChild(empty);
                } else {
                    options.forEach((name) => {
                        const btn = document.createElement('button');
                        btn.className = 'pill' + (state.currentProfile === name ? ' active' : '');
                        btn.textContent = name;
                        btn.dataset.action = 'setProfile';
                        btn.dataset.value = name;
                        profileWrap.appendChild(btn);
                    });
                }
            }

            const strictToggle = document.getElementById('strictToggle');
            if (strictToggle) {
                strictToggle.checked = !!state.settings.strict;
            }
            const repoToggle = document.getElementById('repoToggle');
            if (repoToggle) {
                repoToggle.checked = !!state.settings.noRepoChecks;
            }

            const skillRows = document.querySelectorAll('[data-skill][data-env]');
            skillRows.forEach((row) => {
                const kind = row.getAttribute('data-skill');
                const env = row.getAttribute('data-env');
                const installed =
                    kind &&
                    env &&
                    state.skills &&
                    state.skills[kind] &&
                    state.skills[kind][env];
                const badge = row.querySelector('[data-skill-badge]');
                const button = row.querySelector('button');
                if (badge) {
                    badge.textContent = installed ? 'Installed' : 'Not installed';
                    badge.classList.toggle('ok', !!installed);
                }
                if (button) {
                    button.textContent = installed ? 'Reinstall' : 'Install';
                }
            });
        }

        render(state);

        window.addEventListener('message', (event) => {
            const message = event.data;
            if (message && message.type === 'state') {
                render(message.state);
            }
        });

        document.addEventListener('click', (event) => {
            const target = event.target;
            if (!(target instanceof HTMLElement)) {
        return;
    }
            const actionEl = target.closest('[data-action]');
            if (!actionEl) {
                return;
            }
            const action = actionEl.dataset.action;
            if (!action) {
        return;
    }

            if (action === 'addIgnore') {
                const input = document.getElementById('ignoreInput');
                if (input) {
                    const value = input.value || '';
                    vscode.postMessage({ type: 'addIgnore', value });
                    input.value = '';
                }
                return;
            }

            if (action === 'removeIgnore') {
                const glob = actionEl.dataset.glob || '';
                vscode.postMessage({ type: 'removeIgnore', glob });
                return;
            }

            if (action === 'setProfile') {
                const value = actionEl.dataset.value || '';
                vscode.postMessage({ type: 'setProfile', value });
                return;
            }

            if (action === 'installSkill') {
                vscode.postMessage({
                    type: 'installSkill',
                    env: actionEl.dataset.env,
                    kind: actionEl.dataset.kind,
                });
                return;
            }

            vscode.postMessage({ type: action });
        });

        // Category toggle handlers
        document.getElementById('categoryToggles')?.addEventListener('change', (event) => {
            const input = event.target;
            if (input instanceof HTMLInputElement && input.dataset.category) {
                vscode.postMessage({
                    type: 'toggleCategory',
                    category: input.dataset.category,
                    enabled: input.checked
                });
            }
        });

        // File type toggle handlers
        document.getElementById('fileTypeToggles')?.addEventListener('change', (event) => {
            const input = event.target;
            if (input instanceof HTMLInputElement && input.dataset.filetype) {
                vscode.postMessage({
                    type: 'toggleFileType',
                    fileType: input.dataset.filetype,
                    enabled: input.checked
                });
            }
        });

        document.getElementById('strictToggle')?.addEventListener('change', (event) => {
            const input = event.target;
            if (input instanceof HTMLInputElement) {
                vscode.postMessage({ type: 'toggleSetting', key: 'strict', value: input.checked });
            }
        });

        document.getElementById('repoToggle')?.addEventListener('change', (event) => {
            const input = event.target;
            if (input instanceof HTMLInputElement) {
                vscode.postMessage({ type: 'toggleSetting', key: 'noRepoChecks', value: input.checked });
            }
        });
    </script>
</body>
</html>`;
    }
}

/**
 * Returns the platform-specific subdirectory name for bundled binaries.
 * Maps Node.js os.platform() and os.arch() to our binary directory structure.
 */
function getPlatformDir(): string {
    const platform = os.platform();
    const arch = os.arch();

    if (platform === 'win32') {
        return 'win32-x64';
    } else if (platform === 'darwin') {
        return arch === 'arm64' ? 'darwin-arm64' : 'darwin-x64';
    } else if (platform === 'linux') {
        return arch === 'arm64' ? 'linux-arm64' : 'linux-x64';
    }
    
    // Fallback for unknown platforms
    return `${platform}-${arch}`;
}

/**
 * Gets the path to the bundled LSP server binary.
 * Returns undefined if the bundled binary doesn't exist.
 */
function getBundledServerPath(context: vscode.ExtensionContext): string | undefined {
    const platformDir = getPlatformDir();
    const binaryName = os.platform() === 'win32' ? 'dwg-lsp.exe' : 'dwg-lsp';
    const bundledPath = path.join(context.extensionPath, 'bin', platformDir, binaryName);
    
    if (fs.existsSync(bundledPath)) {
        return bundledPath;
    }
    return undefined;
}

/**
 * Gets the path to the configuration file.
 * Priority:
 * 1. User-specified configPath setting (relative to workspace)
 * 2. Workspace root layth-style.yml
 * 3. Bundled default config
 */
function getConfigPath(context: vscode.ExtensionContext, userConfigPath: string): string {
    // Check if user specified a custom config path
    if (userConfigPath) {
        const workspaceFolders = vscode.workspace.workspaceFolders;
        if (workspaceFolders && workspaceFolders.length > 0) {
            const workspaceConfig = path.join(workspaceFolders[0].uri.fsPath, userConfigPath);
            if (fs.existsSync(workspaceConfig)) {
                return workspaceConfig;
            }
        }
    }
    
    // Check for layth-style.yml in workspace root
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (workspaceFolders && workspaceFolders.length > 0) {
        const defaultNames = ['layth-style.yml', 'layth-style.yaml', '.toneguard.yml', '.toneguard.yaml'];
        for (const name of defaultNames) {
            const workspaceConfig = path.join(workspaceFolders[0].uri.fsPath, name);
            if (fs.existsSync(workspaceConfig)) {
                return workspaceConfig;
            }
        }
    }
    
    // Fall back to bundled config
    const bundledConfig = path.join(context.extensionPath, 'config', 'layth-style.yml');
    if (fs.existsSync(bundledConfig)) {
        return bundledConfig;
    }
    
    // Last resort: return the user config path and let LSP handle the error
    return userConfigPath || 'layth-style.yml';
}

function extractProfilesFromConfig(configPath: string): string[] {
    try {
        const text = fs.readFileSync(configPath, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'profiles');
        if (block.start === -1 || block.end === -1) {
            return [];
        }
        const profiles: string[] = [];
        for (let i = block.start + 1; i < block.end; i += 1) {
            const line = lines[i];
            const match = line.match(/^\s*-\s*name:\s*(.+)$/);
            if (match) {
                let name = match[1].trim();
                name = name.replace(/^["']|["']$/g, '');
                if (name.length > 0) {
                    profiles.push(name);
                }
            }
        }
        return profiles;
    } catch {
        return [];
    }
}

// All known categories in ToneGuard
const ALL_CATEGORIES = [
    'buzzword', 'puffery', 'confidence', 'filler', 'transition',
    'hedge', 'emoji', 'cliche', 'passive', 'uniformity', 'repetition'
];

// Common file types for ToneGuard
const ALL_FILE_TYPES = ['.md', '.txt', '.rst', '.ts', '.tsx', '.js', '.jsx', '.py', '.rs'];

function extractIgnoreGlobsFromConfig(configPath: string): string[] {
    try {
        const text = fs.readFileSync(configPath, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'repo_rules');
        if (block.start === -1) {
            return [];
        }
        
        const globs: string[] = [];
        let inIgnoreGlobs = false;
        let ignoreIndent = 0;
        
        for (let i = block.start + 1; i < block.end; i += 1) {
            const line = lines[i];
            const trimmed = line.trim();
            
            if (/^\s*ignore_globs\s*:/.test(line)) {
                inIgnoreGlobs = true;
                ignoreIndent = line.match(/^\s*/)![0].length;
                continue;
            }
            
            if (inIgnoreGlobs) {
                if (trimmed.length === 0 || trimmed.startsWith('#')) {
                    continue;
                }
                const currentIndent = line.match(/^\s*/)![0].length;
                if (currentIndent <= ignoreIndent && /^[A-Za-z0-9_-]+:/.test(trimmed)) {
                    break;
                }
                if (trimmed.startsWith('-')) {
                    let value = trimmed.replace(/^-+/, '').trim();
                    value = value.replace(/^["']|["']$/g, '');
                    if (value.length > 0) {
                        globs.push(value);
                    }
                }
            }
        }
        return globs;
    } catch {
        return [];
    }
}

function extractDisabledCategoriesFromConfig(configPath: string): string[] {
    try {
        const text = fs.readFileSync(configPath, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'disabled_categories');
        if (block.start === -1) {
            return [];
        }
        
        const disabled: string[] = [];
        for (let i = block.start + 1; i < block.end; i += 1) {
            const line = lines[i];
            const trimmed = line.trim();
            if (trimmed.startsWith('-')) {
                let value = trimmed.replace(/^-+/, '').trim();
                value = value.replace(/^["']|["']$/g, '');
                if (value.length > 0) {
                    disabled.push(value.toLowerCase());
                }
            }
        }
        return disabled;
    } catch {
        return [];
    }
}

function extractEnabledFileTypesFromConfig(configPath: string): string[] {
    try {
        const text = fs.readFileSync(configPath, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'file_types');
        if (block.start === -1) {
            // Default to all if not specified
            return ALL_FILE_TYPES;
        }
        
        const types: string[] = [];
        for (let i = block.start + 1; i < block.end; i += 1) {
            const line = lines[i];
            const trimmed = line.trim();
            if (trimmed.startsWith('-')) {
                let value = trimmed.replace(/^-+/, '').trim();
                value = value.replace(/^["']|["']$/g, '');
                if (value.length > 0) {
                    types.push(value.startsWith('.') ? value : `.${value}`);
                }
            }
        }
        return types.length > 0 ? types : ALL_FILE_TYPES;
    } catch {
        return ALL_FILE_TYPES;
    }
}

function findTopLevelBlock(
    lines: string[],
    key: string
): { start: number; end: number } {
    const start = lines.findIndex((line) => line.trim() === `${key}:`);
    if (start === -1) {
        return { start: -1, end: -1 };
    }
    let end = lines.length;
    for (let i = start + 1; i < lines.length; i += 1) {
        const line = lines[i];
        const trimmed = line.trim();
        if (trimmed.length === 0 || trimmed.startsWith('#')) {
            continue;
        }
        if (/^\S/.test(line) && /^[A-Za-z0-9_-]+:/.test(trimmed)) {
            end = i;
            break;
        }
    }
    return { start, end };
}

function addIgnoreGlobToConfigFile(
    configPath: string,
    glob: string
): { updated: boolean; alreadyPresent: boolean } {
    const text = fs.readFileSync(configPath, 'utf8');
    const lines = text.split(/\r?\n/);
    const block = findTopLevelBlock(lines, 'repo_rules');
    const normalizedTarget = glob.trim().replace(/^["']|["']$/g, '');

    const quoted = JSON.stringify(normalizedTarget);

    if (block.start === -1) {
        lines.push('');
        lines.push('repo_rules:');
        lines.push('  ignore_globs:');
        lines.push(`    - ${quoted}`);
        fs.writeFileSync(configPath, lines.join('\n'), 'utf8');
        return { updated: true, alreadyPresent: false };
    }

    let ignoreIdx = -1;
    for (let i = block.start + 1; i < block.end; i += 1) {
        if (/^\s+ignore_globs\s*:/.test(lines[i])) {
            ignoreIdx = i;
            break;
        }
    }

    if (ignoreIdx === -1) {
        lines.splice(block.end, 0, '  ignore_globs:', `    - ${quoted}`);
        fs.writeFileSync(configPath, lines.join('\n'), 'utf8');
        return { updated: true, alreadyPresent: false };
    }

    const ignoreIndent = lines[ignoreIdx].match(/^\s*/)![0].length;
    let insertAt = ignoreIdx + 1;
    for (let i = ignoreIdx + 1; i < block.end; i += 1) {
        const line = lines[i];
        const trimmed = line.trim();
        if (trimmed.length === 0 || trimmed.startsWith('#')) {
            continue;
        }
        const indent = line.match(/^\s*/)![0].length;
        if (indent <= ignoreIndent) {
            break;
        }
        if (trimmed.startsWith('-')) {
            const value = trimmed.replace(/^-+/, '').trim();
            const normalized = value.replace(/^["']|["']$/g, '');
            if (normalized === normalizedTarget) {
                return { updated: false, alreadyPresent: true };
            }
            insertAt = i + 1;
        }
    }

    const itemPrefix = ' '.repeat(ignoreIndent + 2) + '- ';
    lines.splice(insertAt, 0, `${itemPrefix}${quoted}`);
    fs.writeFileSync(configPath, lines.join('\n'), 'utf8');
    return { updated: true, alreadyPresent: false };
}

async function ensureWorkspaceConfig(
    context: vscode.ExtensionContext,
    configPath: string
): Promise<string | undefined> {
    const workspaceRoot = getWorkspaceRoot();
    if (!workspaceRoot) {
        void vscode.window.showErrorMessage('ToneGuard: No workspace open.');
        return undefined;
    }

    const absConfig = resolveConfigPath(workspaceRoot, configPath);
    if (absConfig && fs.existsSync(absConfig) && isPathInside(workspaceRoot, absConfig)) {
        return absConfig;
    }

    const bundledConfig = path.join(context.extensionPath, 'config', 'layth-style.yml');
    if (!fs.existsSync(bundledConfig)) {
        void vscode.window.showErrorMessage('ToneGuard: bundled config not found.');
        return undefined;
    }

    const target = path.join(workspaceRoot, 'layth-style.yml');
    const action = await vscode.window.showInformationMessage(
        'ToneGuard config is not editable here. Create a workspace copy?',
        'Create',
        'Cancel'
    );
    if (action !== 'Create') {
        return undefined;
    }

    try {
        if (!fs.existsSync(target)) {
            fs.copyFileSync(bundledConfig, target);
        }
        const config = vscode.workspace.getConfiguration('dwg');
        await config.update(
            'configPath',
            'layth-style.yml',
            vscode.ConfigurationTarget.Workspace
        );
        return target;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        void vscode.window.showErrorMessage(
            `ToneGuard: failed to create config copy: ${message}`
        );
        return undefined;
    }
}

async function addIgnoreGlobToConfig(
    context: vscode.ExtensionContext,
    configPath: string,
    glob: string,
    outputChannel: vscode.OutputChannel
): Promise<boolean> {
    const resolved = await ensureWorkspaceConfig(context, configPath);
    if (!resolved) {
        return false;
    }

    try {
        const result = addIgnoreGlobToConfigFile(resolved, glob);
        if (result.alreadyPresent) {
            void vscode.window.showInformationMessage(
                'ToneGuard: ignore glob already exists.'
            );
        } else {
            void vscode.window.showInformationMessage(
                'ToneGuard: added ignore glob to repo_rules.'
            );
        }
        return true;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`ToneGuard: failed to update config: ${message}`);
        void vscode.window.showErrorMessage(
            `ToneGuard: failed to update config: ${message}`
        );
        return false;
    }
}

async function openConfigFile(
    context: vscode.ExtensionContext,
    configPath: string
): Promise<void> {
    const workspaceRoot = getWorkspaceRoot();
    const resolved = resolveConfigPath(workspaceRoot, configPath);
    if (resolved && fs.existsSync(resolved)) {
        const doc = await vscode.workspace.openTextDocument(resolved);
        await vscode.window.showTextDocument(doc, { preview: false });
        return;
    }

    const bundledConfig = path.join(context.extensionPath, 'config', 'layth-style.yml');
    if (fs.existsSync(bundledConfig)) {
        const doc = await vscode.workspace.openTextDocument(bundledConfig);
        await vscode.window.showTextDocument(doc, { preview: false });
        return;
    }

    void vscode.window.showErrorMessage('ToneGuard: config file not found.');
}

async function toggleCategoryInConfig(
    context: vscode.ExtensionContext,
    configPath: string,
    category: string,
    enabled: boolean,
    outputChannel: vscode.OutputChannel
): Promise<boolean> {
    const resolved = await ensureWorkspaceConfig(context, configPath);
    if (!resolved) {
        return false;
    }

    try {
        const text = fs.readFileSync(resolved, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'disabled_categories');
        const normalizedCat = category.toLowerCase().trim();

        if (enabled) {
            // Remove from disabled_categories
            if (block.start === -1) {
                return true; // Not disabled, nothing to do
            }
            const newLines = lines.filter((line, idx) => {
                if (idx <= block.start || idx >= block.end) return true;
                const trimmed = line.trim();
                if (trimmed.startsWith('-')) {
                    const val = trimmed.replace(/^-+/, '').trim().replace(/^["']|["']$/g, '').toLowerCase();
                    return val !== normalizedCat;
                }
                return true;
            });
            fs.writeFileSync(resolved, newLines.join('\n'), 'utf8');
        } else {
            // Add to disabled_categories
            if (block.start === -1) {
                // Create disabled_categories section
                lines.push('');
                lines.push('disabled_categories:');
                lines.push(`  - ${JSON.stringify(normalizedCat)}`);
            } else {
                // Check if already present
                for (let i = block.start + 1; i < block.end; i++) {
                    const trimmed = lines[i].trim();
                    if (trimmed.startsWith('-')) {
                        const val = trimmed.replace(/^-+/, '').trim().replace(/^["']|["']$/g, '').toLowerCase();
                        if (val === normalizedCat) {
                            return true; // Already disabled
                        }
                    }
                }
                // Add to section
                lines.splice(block.start + 1, 0, `  - ${JSON.stringify(normalizedCat)}`);
            }
            fs.writeFileSync(resolved, lines.join('\n'), 'utf8');
        }
        return true;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`ToneGuard: failed to update config: ${message}`);
        return false;
    }
}

async function removeIgnoreGlobFromConfig(
    context: vscode.ExtensionContext,
    configPath: string,
    glob: string,
    outputChannel: vscode.OutputChannel
): Promise<boolean> {
    const resolved = await ensureWorkspaceConfig(context, configPath);
    if (!resolved) {
        return false;
    }

    try {
        const text = fs.readFileSync(resolved, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'repo_rules');
        if (block.start === -1) {
            return true;
        }

        const normalizedGlob = glob.trim().replace(/^["']|["']$/g, '');
        let inIgnoreGlobs = false;
        let ignoreIndent = 0;

        const newLines = lines.filter((line, idx) => {
            if (idx < block.start || idx >= block.end) return true;
            
            if (/^\s*ignore_globs\s*:/.test(line)) {
                inIgnoreGlobs = true;
                ignoreIndent = line.match(/^\s*/)![0].length;
                return true;
            }
            
            if (inIgnoreGlobs) {
                const trimmed = line.trim();
                const currentIndent = line.match(/^\s*/)![0].length;
                if (currentIndent <= ignoreIndent && /^[A-Za-z0-9_-]+:/.test(trimmed)) {
                    inIgnoreGlobs = false;
                    return true;
                }
                if (trimmed.startsWith('-')) {
                    const val = trimmed.replace(/^-+/, '').trim().replace(/^["']|["']$/g, '');
                    if (val === normalizedGlob) {
                        return false; // Remove this line
                    }
                }
            }
            return true;
        });

        fs.writeFileSync(resolved, newLines.join('\n'), 'utf8');
        void vscode.window.showInformationMessage(`ToneGuard: removed ignore glob "${glob}"`);
        return true;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`ToneGuard: failed to update config: ${message}`);
        return false;
    }
}

async function toggleFileTypeInConfig(
    context: vscode.ExtensionContext,
    configPath: string,
    fileType: string,
    enabled: boolean,
    outputChannel: vscode.OutputChannel
): Promise<boolean> {
    const resolved = await ensureWorkspaceConfig(context, configPath);
    if (!resolved) {
        return false;
    }

    try {
        const text = fs.readFileSync(resolved, 'utf8');
        const lines = text.split(/\r?\n/);
        const block = findTopLevelBlock(lines, 'file_types');
        const normalizedType = fileType.startsWith('.') ? fileType : `.${fileType}`;

        if (block.start === -1) {
            // Create file_types section with all types except the disabled one (if disabling)
            const types = enabled 
                ? [...ALL_FILE_TYPES]
                : ALL_FILE_TYPES.filter(t => t !== normalizedType);
            lines.push('');
            lines.push('file_types:');
            for (const t of types) {
                lines.push(`  - ${JSON.stringify(t)}`);
            }
            fs.writeFileSync(resolved, lines.join('\n'), 'utf8');
            return true;
        }

        if (enabled) {
            // Add to file_types if not present
            let found = false;
            for (let i = block.start + 1; i < block.end; i++) {
                const trimmed = lines[i].trim();
                if (trimmed.startsWith('-')) {
                    let val = trimmed.replace(/^-+/, '').trim().replace(/^["']|["']$/g, '');
                    val = val.startsWith('.') ? val : `.${val}`;
                    if (val === normalizedType) {
                        found = true;
                        break;
                    }
                }
            }
            if (!found) {
                lines.splice(block.start + 1, 0, `  - ${JSON.stringify(normalizedType)}`);
                fs.writeFileSync(resolved, lines.join('\n'), 'utf8');
            }
        } else {
            // Remove from file_types
            const newLines = lines.filter((line, idx) => {
                if (idx <= block.start || idx >= block.end) return true;
                const trimmed = line.trim();
                if (trimmed.startsWith('-')) {
                    let val = trimmed.replace(/^-+/, '').trim().replace(/^["']|["']$/g, '');
                    val = val.startsWith('.') ? val : `.${val}`;
                    return val !== normalizedType;
                }
                return true;
            });
            fs.writeFileSync(resolved, newLines.join('\n'), 'utf8');
        }
        return true;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`ToneGuard: failed to update config: ${message}`);
        return false;
    }
}

/**
 * Generates a fix prompt for AI agents from findings.
 */
function generateFixPrompt(findings: any[]): string {
    const parts: string[] = [
        '# ToneGuard: Fix These Issues\n',
        'Please fix the following issues found by ToneGuard:\n'
    ];
    
    for (const finding of findings) {
        const file = finding.path || finding.file || '<unknown>';
        const line = finding.line || '?';
        const message = finding.message || 'Issue found';
        const fix = finding.fix_instructions;
        
        parts.push(`\n## ${file}:${line}\n`);
        parts.push(`**Issue**: ${message}\n`);
        
        if (fix) {
            if (typeof fix === 'object') {
                parts.push(`**Action**: ${fix.action || 'fix'}\n`);
                if (fix.description) parts.push(`**How**: ${fix.description}\n`);
                if (fix.find_pattern) parts.push(`**Find**: \`${fix.find_pattern}\`\n`);
                if (fix.replace_pattern) parts.push(`**Replace**: \`${fix.replace_pattern}\`\n`);
                if (fix.alternative) parts.push(`**Alternative**: ${fix.alternative}\n`);
            } else {
                parts.push(`**Fix**: ${fix}\n`);
            }
        }
    }
    
    parts.push('\n---\nApply these fixes to the codebase.\n');
    return parts.join('');
}

/**
 * Fix findings using an AI agent (Cursor Composer, Claude, or Codex).
 */
async function fixWithAI(
    context: vscode.ExtensionContext,
    findings: any[],
    env: AIEnvironment,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const prompt = generateFixPrompt(findings);
    
    // Copy to clipboard
    await vscode.env.clipboard.writeText(prompt);
    outputChannel.appendLine(`ToneGuard: Copied ${findings.length} fix instructions to clipboard`);
    
    if (env === 'cursor') {
        // Try to open Cursor Composer
        try {
            await vscode.commands.executeCommand('cursor.composer.new');
        void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied. Paste in Composer to apply.`
            );
        } catch {
            void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied to clipboard. Open Composer (Cmd+I) and paste.`
            );
        }
    } else {
        // For Claude/Codex, just inform user
        void vscode.window.showInformationMessage(
            `ToneGuard: ${findings.length} fix instructions copied. Paste in ${getEnvDisplayName(env)} to apply.`
        );
    }
}

/**
 * Resolves the server command to use.
 * Priority:
 * 1. User-specified command in settings
 * 2. Bundled binary for current platform
 * 3. 'dwg-lsp' from PATH
 */
function getServerCommand(context: vscode.ExtensionContext, userCommand: string): string {
    // If user explicitly set a command (and it's not the default), use it
    if (userCommand && userCommand !== 'dwg-lsp') {
        return userCommand;
    }
    
    // Try bundled binary first
    const bundledPath = getBundledServerPath(context);
    if (bundledPath) {
        // Make sure it's executable on Unix
        if (os.platform() !== 'win32') {
            try {
                fs.chmodSync(bundledPath, 0o755);
            } catch {
                // Ignore chmod errors (might already be executable or read-only)
            }
        }
        return bundledPath;
    }
    
    // Fall back to PATH
    return 'dwg-lsp';
}

/**
 * Gets the path to the bundled CLI binary.
 */
function getBundledCliPath(context: vscode.ExtensionContext): string | undefined {
    const platformDir = getPlatformDir();
    const binaryName = os.platform() === 'win32' ? 'dwg.exe' : 'dwg';
    const bundledPath = path.join(context.extensionPath, 'bin', platformDir, binaryName);
    if (fs.existsSync(bundledPath)) {
        return bundledPath;
    }
    return undefined;
}

/**
 * Resolves the CLI command to use.
 * Priority:
 * 1. User-specified command in settings
 * 2. Bundled binary for current platform
 * 3. 'dwg' from PATH
 */
function getCliCommand(context: vscode.ExtensionContext, userCommand: string): string {
    if (userCommand && userCommand !== 'dwg') {
        return userCommand;
    }
    const bundledPath = getBundledCliPath(context);
    if (bundledPath) {
        if (os.platform() !== 'win32') {
            try {
                fs.chmodSync(bundledPath, 0o755);
            } catch {
                // Ignore chmod errors
            }
        }
        return bundledPath;
    }
    return 'dwg';
}

/**
 * Detects all AI environments: Cursor IDE, Claude Code, and Codex.
 * - Cursor: Detected by checking if running in Cursor IDE
 * - Claude Code: Detected by .claude directory in workspace
 * - Codex: Detected by .codex directory in workspace
 */
function detectAIEnvironments(): DetectedEnvironments {
    const isCursor = vscode.env.appName.toLowerCase().includes('cursor');
    
    const workspaceFolders = vscode.workspace.workspaceFolders;
    let hasClaude = false;
    let hasCodex = false;
    
    if (workspaceFolders && workspaceFolders.length > 0) {
        const workspaceRoot = workspaceFolders[0].uri.fsPath;
        hasClaude = fs.existsSync(path.join(workspaceRoot, '.claude'));
        hasCodex = fs.existsSync(path.join(workspaceRoot, '.codex'));
    }

    return {
        cursor: isCursor,
        claude: hasClaude,
        codex: hasCodex,
    };
}

/**
 * Legacy function for backward compatibility.
 * Returns the first detected AI environment or null.
 */
async function detectAIEnvironment(): Promise<AIEnvironment | null> {
    const envs = detectAIEnvironments();
    if (envs.claude) return 'claude';
    if (envs.codex) return 'codex';
    if (envs.cursor) return 'cursor';
    return null;
}

/**
 * Gets the global skill directory for the given AI environment.
 * Skills are installed globally so they apply across all projects.
 */
function getGlobalSkillDir(env: AIEnvironment, kind: SkillKind): string {
    const homeDir = os.homedir();
    let baseDir: string;
    if (env === 'cursor') {
        baseDir = '.cursor';
    } else if (env === 'claude') {
        baseDir = '.claude';
    } else {
        baseDir = '.codex';
    }
    return path.join(homeDir, baseDir, 'skills', SKILL_META[kind].dirName);
}

/**
 * Checks if the ToneGuard skill is already installed globally for the given environment.
 */
function isSkillInstalled(env: AIEnvironment, kind: SkillKind): boolean {
    const skillDir = getGlobalSkillDir(env, kind);
    return fs.existsSync(path.join(skillDir, 'SKILL.md'));
}

/**
 * Gets the bundled skill template content.
 */
function getSkillTemplate(
    context: vscode.ExtensionContext,
    kind: SkillKind
): string | undefined {
    const templatePath = path.join(
        context.extensionPath,
        'skills',
        SKILL_META[kind].template
    );
    if (fs.existsSync(templatePath)) {
        return fs.readFileSync(templatePath, 'utf8');
    }
    return undefined;
}

/**
 * Gets human-readable name for an AI environment.
 */
function getEnvDisplayName(env: AIEnvironment): string {
    if (env === 'cursor') return 'Cursor';
    if (env === 'claude') return 'Claude Code';
    return 'Codex';
}

/**
 * Installs the ToneGuard skill globally for the specified AI environment.
 * Skills are installed in ~/.cursor/skills/, ~/.claude/skills/, or ~/.codex/skills/
 * so they apply across all projects.
 */
async function installSkill(
    context: vscode.ExtensionContext, 
    env: AIEnvironment,
    outputChannel: vscode.OutputChannel,
    kind: SkillKind
): Promise<boolean> {
    const skillTemplate = getSkillTemplate(context, kind);
    
    if (!skillTemplate) {
        void vscode.window.showErrorMessage(
            `ToneGuard: Could not find ${SKILL_META[kind].label} template. Please reinstall the extension.`
        );
        return false;
    }

    // Install to global directory
    const skillDir = getGlobalSkillDir(env, kind);
    const skillFile = path.join(skillDir, 'SKILL.md');

    try {
        // Create directory structure
        fs.mkdirSync(skillDir, { recursive: true });
        
        // Write the skill file
        fs.writeFileSync(skillFile, skillTemplate, 'utf8');
        
        outputChannel.appendLine(`ToneGuard: Installed ${SKILL_META[kind].label} to ${skillFile}`);
        
        void vscode.window.showInformationMessage(
            `${SKILL_META[kind].label} installed for ${getEnvDisplayName(env)}! ` +
            SKILL_META[kind].success,
            'View Skill'
        ).then((selection) => {
            if (selection === 'View Skill') {
                void vscode.workspace.openTextDocument(skillFile).then((doc) => {
                    void vscode.window.showTextDocument(doc);
                });
            }
        });
        
        return true;
    } catch (err) {
        const errorMessage = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`ToneGuard: Failed to install skill: ${errorMessage}`);
        void vscode.window.showErrorMessage(
            `ToneGuard: Failed to install skill: ${errorMessage}`
        );
        return false;
    }
}

/**
 * Prompts the user to install ToneGuard skills for detected AI environments.
 * Checks Cursor, Claude Code, and Codex, and prompts for any missing skills.
 */
async function promptSkillInstallationForAll(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    // Check if user has dismissed this prompt before
    const dismissed = context.globalState.get<boolean>(SKILL_PROMPT_DISMISSED_KEY, false);
    if (dismissed) {
        return;
    }

    const envs = detectAIEnvironments();
    const missingEnvs: AIEnvironment[] = [];

    // Check which detected environments are missing skills
    if (envs.cursor && !isSkillInstalled('cursor', 'writing')) {
        missingEnvs.push('cursor');
    }
    if (envs.claude && !isSkillInstalled('claude', 'writing')) {
        missingEnvs.push('claude');
    }
    if (envs.codex && !isSkillInstalled('codex', 'writing')) {
        missingEnvs.push('codex');
    }

    if (missingEnvs.length === 0) {
        return;
    }

    const envNames = missingEnvs.map(getEnvDisplayName).join(', ');
    const action = await vscode.window.showInformationMessage(
        `ToneGuard detected: ${envNames}. Install skills to prevent AI slop?`,
        'Install All',
        'Choose...',
        'Not Now',
        'Never Ask'
    );

    if (action === 'Install All') {
        for (const env of missingEnvs) {
            await installSkill(context, env, outputChannel, 'writing');
        }
    } else if (action === 'Choose...') {
        const choices = missingEnvs.map(env => ({
            label: getEnvDisplayName(env),
            value: env,
            picked: true
        }));
        const selected = await vscode.window.showQuickPick(choices, {
            canPickMany: true,
            placeHolder: 'Select environments to install skills for'
        });
        if (selected) {
            for (const choice of selected) {
                await installSkill(context, choice.value, outputChannel, 'writing');
            }
        }
    } else if (action === 'Never Ask') {
        await context.globalState.update(SKILL_PROMPT_DISMISSED_KEY, true);
        outputChannel.appendLine('ToneGuard: Skill installation prompt dismissed permanently.');
    }
}

function getNonce(): string {
    let text = '';
    const possible =
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    for (let i = 0; i < 32; i += 1) {
        text += possible.charAt(Math.floor(Math.random() * possible.length));
    }
    return text;
}

export function activate(context: vscode.ExtensionContext): void {
    const config = vscode.workspace.getConfiguration('dwg');
    const userCommand = config.get<string>('command', 'dwg-lsp');
    const userCliCommand = config.get<string>('cliCommand', '');
    const userConfigPath = config.get<string>('configPath', 'layth-style.yml');
    let profile = config.get<string>('profile', '').trim();

    // Resolve actual paths
    const command = getServerCommand(context, userCommand);
    let cliCommand = getCliCommand(context, userCliCommand);
    let configPath = getConfigPath(context, userConfigPath);
    
    // Log which server we're using for debugging
    const outputChannel = vscode.window.createOutputChannel('ToneGuard');
    context.subscriptions.push(outputChannel);
    outputChannel.appendLine(`ToneGuard: Using server: ${command}`);
    outputChannel.appendLine(`ToneGuard: Using CLI: ${cliCommand}`);
    outputChannel.appendLine(`ToneGuard: Using config: ${configPath}`);

    const sidebarProvider = new ToneGuardSidebarProvider();
    context.subscriptions.push(
        vscode.window.registerTreeDataProvider('toneguard.sidebar', sidebarProvider)
    );

    const dashboardProvider = new ToneGuardDashboardProvider(
        context,
        outputChannel,
        command,
        () => ({ cliCommand, configPath, profile })
    );
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            'toneguard.dashboard',
            dashboardProvider
        )
    );

    const refreshAll = (): void => {
        sidebarProvider.refresh();
        void dashboardProvider.refresh();
    };
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.refreshSidebar', () => {
            refreshAll();
        })
    );
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.openDocs', async () => {
            void vscode.env.openExternal(
                vscode.Uri.parse('https://github.com/editnori/toneguard#readme')
            );
        })
    );
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.openFlowAuditReport', async () => {
            const root = getWorkspaceRoot();
            if (!root) {
                return;
            }
            const reportPath = path.join(root, 'reports', 'flow-audit.json');
            if (!fs.existsSync(reportPath)) {
                void vscode.window.showErrorMessage(
                    'ToneGuard: reports/flow-audit.json not found. Run Flow Audit first.'
                );
                return;
            }
            const doc = await vscode.workspace.openTextDocument(reportPath);
            await vscode.window.showTextDocument(doc, { preview: false });
        })
    );
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.openFlowProposalReport', async () => {
            const root = getWorkspaceRoot();
            if (!root) {
                return;
            }
            const reportPath = path.join(root, 'reports', 'flow-proposal.md');
            if (!fs.existsSync(reportPath)) {
                void vscode.window.showErrorMessage(
                    'ToneGuard: reports/flow-proposal.md not found. Generate a proposal first.'
                );
                return;
            }
            const doc = await vscode.workspace.openTextDocument(reportPath);
            await vscode.window.showTextDocument(doc, { preview: false });
        })
    );
    context.subscriptions.push(
        vscode.commands.registerCommand(
            'dwg.openFindingLocation',
            async (reportedPath?: string, line?: number) => {
                const root = getWorkspaceRoot();
                if (!root || !reportedPath) {
                    return;
                }
                const cleaned = normalizeReportedPath(String(reportedPath));
                const abs = path.isAbsolute(cleaned)
                    ? cleaned
                    : path.join(root, cleaned);
                if (!fs.existsSync(abs)) {
                    void vscode.window.showErrorMessage(
                        `ToneGuard: file not found: ${cleaned}`
                    );
                    return;
                }
                const doc = await vscode.workspace.openTextDocument(abs);
                const editor = await vscode.window.showTextDocument(doc, {
                    preview: false,
                });
                if (typeof line === 'number' && line > 0) {
                    const pos = new vscode.Position(line - 1, 0);
                    editor.selection = new vscode.Selection(pos, pos);
                    editor.revealRange(new vscode.Range(pos, pos));
                }
            }
        )
    );

    const auditWatcher = vscode.workspace.createFileSystemWatcher(
        '**/reports/flow-audit.json'
    );
    auditWatcher.onDidCreate(() => refreshAll());
    auditWatcher.onDidChange(() => refreshAll());
    auditWatcher.onDidDelete(() => refreshAll());
    context.subscriptions.push(auditWatcher);

    const proposalWatcher = vscode.workspace.createFileSystemWatcher(
        '**/reports/flow-proposal.md'
    );
    proposalWatcher.onDidCreate(() => refreshAll());
    proposalWatcher.onDidChange(() => refreshAll());
    proposalWatcher.onDidDelete(() => refreshAll());
    context.subscriptions.push(proposalWatcher);

    const flowsWatcher = vscode.workspace.createFileSystemWatcher('**/flows/*');
    flowsWatcher.onDidCreate(() => refreshAll());
    flowsWatcher.onDidChange(() => refreshAll());
    flowsWatcher.onDidDelete(() => refreshAll());
    context.subscriptions.push(flowsWatcher);

    context.subscriptions.push(
        vscode.workspace.onDidChangeConfiguration((event) => {
            if (!event.affectsConfiguration('dwg')) {
                return;
            }
            const config = vscode.workspace.getConfiguration('dwg');
            const nextCli = getCliCommand(
                context,
                config.get<string>('cliCommand', '')
            );
            const nextConfigPath = getConfigPath(
                context,
                config.get<string>('configPath', 'layth-style.yml')
            );
            const nextProfile = config.get<string>('profile', '').trim();

            cliCommand = nextCli;
            configPath = nextConfigPath;
            profile = nextProfile;
            refreshAll();
        })
    );

    const serverOptions: ServerOptions = {
        command,
        args: [],
    };

    const fileEvents: vscode.FileSystemWatcher[] = [];
    // Watch for config file changes
    if (configPath && !path.isAbsolute(configPath)) {
        fileEvents.push(vscode.workspace.createFileSystemWatcher(`**/${configPath}`));
    } else if (configPath) {
        // For absolute paths, watch the specific file
        fileEvents.push(vscode.workspace.createFileSystemWatcher(configPath));
    }

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'markdown' },
            { scheme: 'file', language: 'plaintext' },
        ],
        initializationOptions: {
            configPath,
            profile,
        },
        synchronize: {
            configurationSection: 'dwg',
            fileEvents,
        },
        outputChannel,
    };

    client = new LanguageClient(
        'toneguard',
        'ToneGuard Language Server',
        serverOptions,
        clientOptions
    );

    context.subscriptions.push({
        dispose: () => {
            void client?.stop();
        },
    });
    
    // Start the client and handle errors gracefully
    client.start().catch((err: Error) => {
        const bundledPath = getBundledServerPath(context);
        if (!bundledPath) {
            void vscode.window.showErrorMessage(
                `ToneGuard: Could not start language server. ` +
                `No bundled binary found for your platform (${getPlatformDir()}). ` +
                `Please install dwg-lsp manually: cargo install --path lsp`,
                'Install Instructions'
            ).then((selection) => {
                if (selection === 'Install Instructions') {
                    void vscode.env.openExternal(
                        vscode.Uri.parse('https://github.com/editnori/toneguard#installation')
                    );
                }
            });
        } else {
            void vscode.window.showErrorMessage(
                `ToneGuard: Failed to start language server: ${err.message}`
            );
        }
        outputChannel.appendLine(`Error starting server: ${err.message}`);
    });

    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.lintWorkspace', async () => {
            if (!client) {
                return;
            }
            await client.sendNotification('workspace/didChangeConfiguration', { settings: {} });
            void vscode.window.showInformationMessage(
                'ToneGuard: refreshed diagnostics for open files.'
            );
        })
    );

    // One-click recommended workflow: audit + proposal (writes both reports)
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.runRecommended', async () => {
            const workspaceRoot = getWorkspaceRoot();
            if (!workspaceRoot) {
                void vscode.window.showErrorMessage('ToneGuard: No workspace open.');
                return;
            }

            const auditPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
            const proposalPath = path.join(
                workspaceRoot,
                'reports',
                'flow-proposal.md'
            );

            function runCli(args: string[], title: string): Promise<void> {
    return new Promise((resolve, reject) => {
                    outputChannel.show(true);
                    outputChannel.appendLine(`ToneGuard: ${title}`);
                    outputChannel.appendLine(`ToneGuard: ${cliCommand} ${args.join(' ')}`);
                    execFile(
                        cliCommand,
                        args,
                        { cwd: workspaceRoot },
                        (error, stdout, stderr) => {
                            if (stdout) {
                                outputChannel.appendLine(stdout);
                            }
                            if (stderr) {
                                outputChannel.appendLine(stderr);
                            }
                            if (error) {
                                reject(error);
                                return;
                            }
                            resolve();
                        }
                    );
                });
            }

            try {
                await vscode.window.withProgress(
                    {
                        location: vscode.ProgressLocation.Notification,
                        title: 'ToneGuard: Running recommended review',
                        cancellable: false,
                    },
                    async (progress) => {
                        progress.report({ message: 'Running flow audit…' });
                        await runCli(
                            [
                                'flow',
                                'audit',
                                '--config',
                                configPath,
                                '--out',
                                auditPath,
                                workspaceRoot,
                            ],
                            'Running flow audit…'
                        );

                        progress.report({ message: 'Generating flow proposal…' });
                        await runCli(
                            [
                                'flow',
                                'propose',
                                '--config',
                                configPath,
                                '--out',
                                proposalPath,
                                workspaceRoot,
                            ],
                            'Generating flow proposal…'
                        );
                    }
                );

                refreshAll();
                
                // Read audit results to get finding count
                let findingCount = 0;
                let findings: any[] = [];
                try {
                    const auditText = fs.readFileSync(auditPath, 'utf8');
                    const auditData = JSON.parse(auditText);
                    findings = Array.isArray(auditData?.audit?.findings) ? auditData.audit.findings : [];
                    findingCount = findings.length;
                } catch {
                    // Ignore parse errors
                }

                if (findingCount === 0) {
                    void vscode.window.showInformationMessage(
                        'ToneGuard: All clear! No issues found. 🎉'
                    );
                    return;
                }

                // Build fix options based on detected environments
                const envs = detectAIEnvironments();
                const fixOptions: string[] = [];
                if (envs.cursor) fixOptions.push('Fix with Cursor');
                if (envs.claude) fixOptions.push('Fix with Claude');
                if (envs.codex) fixOptions.push('Fix with Codex');
                fixOptions.push('Review Proposal');

                const choice = await vscode.window.showWarningMessage(
                    `ToneGuard: ${findingCount} issue(s) found`,
                    ...fixOptions,
                    'Dismiss'
                );

                if (choice === 'Fix with Cursor') {
                    const prompt = generateFixPrompt(findings);
                    await vscode.env.clipboard.writeText(prompt);
                    try {
                        await vscode.commands.executeCommand('cursor.composer.new');
                        void vscode.window.showInformationMessage(
                            'ToneGuard: Fix instructions copied. Paste in Composer to apply.'
                        );
                    } catch {
                        void vscode.window.showInformationMessage(
                            'ToneGuard: Fix instructions copied. Open Composer (Cmd+I) and paste.'
                        );
                    }
                } else if (choice === 'Fix with Claude') {
                    const prompt = generateFixPrompt(findings);
                    await vscode.env.clipboard.writeText(prompt);
                    void vscode.window.showInformationMessage(
                        'ToneGuard: Fix instructions copied. Paste in Claude Code to apply.'
                    );
                } else if (choice === 'Fix with Codex') {
                    const prompt = generateFixPrompt(findings);
                    await vscode.env.clipboard.writeText(prompt);
                    void vscode.window.showInformationMessage(
                        'ToneGuard: Fix instructions copied. Paste in Codex to apply.'
                    );
                } else if (choice === 'Review Proposal') {
                    const doc = await vscode.workspace.openTextDocument(proposalPath);
                    await vscode.window.showTextDocument(doc, { preview: false });
                }
            } catch (error) {
                const err = error as NodeJS.ErrnoException;
                const message = err.message || String(err);
                outputChannel.appendLine(
                    `ToneGuard: Recommended run failed: ${message}`
                );
                if (err.code === 'ENOENT') {
                    void vscode.window.showErrorMessage(
                        'ToneGuard CLI not found for this platform. Bundle it (scripts/install-local.sh) or set dwg.cliCommand.',
                        'Install Instructions'
                    );
                } else {
                    void vscode.window.showErrorMessage(
                        'ToneGuard: Recommended run failed. See output for details.'
                    );
                }
            }
        })
    );

    // Command to run flow audit via the CLI
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.flowAudit', async () => {
            const workspaceFolders = vscode.workspace.workspaceFolders;
            if (!workspaceFolders || workspaceFolders.length === 0) {
                void vscode.window.showErrorMessage('ToneGuard: No workspace open.');
                return;
            }
            const workspaceRoot = workspaceFolders[0].uri.fsPath;
            const reportPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
            const args = ['flow', 'audit', '--config', configPath, '--out', reportPath, workspaceRoot];

            outputChannel.show(true);
            outputChannel.appendLine(`ToneGuard: Running flow audit...`);
            outputChannel.appendLine(`ToneGuard: ${cliCommand} ${args.join(' ')}`);

            execFile(cliCommand, args, { cwd: workspaceRoot }, (error, stdout, stderr) => {
                if (stdout) {
                    outputChannel.appendLine(stdout);
                }
                if (stderr) {
                    outputChannel.appendLine(stderr);
                }
                if (error) {
                    const err = error as NodeJS.ErrnoException;
                    const message = err.message || String(err);
                    outputChannel.appendLine(`ToneGuard: Flow audit failed: ${message}`);
                    if (err.code === 'ENOENT') {
                        void vscode.window.showErrorMessage(
                            'ToneGuard CLI not found. Install with `cargo install --path cli` or bundle the CLI.',
                            'Install Instructions'
                        ).then((selection) => {
                            if (selection === 'Install Instructions') {
                                void vscode.env.openExternal(
                                    vscode.Uri.parse('https://github.com/editnori/toneguard#installation')
                                );
                            }
                        });
                    } else {
                        void vscode.window.showErrorMessage(
                            `ToneGuard flow audit failed. See output for details.`
                        );
                    }
                    return;
                }
                outputChannel.appendLine(`ToneGuard: Flow audit report saved to ${reportPath}`);
                void vscode.window.showInformationMessage(
                    'ToneGuard: Flow audit complete.',
                    'Open Report'
                ).then((selection) => {
                    if (selection === 'Open Report') {
                        void vscode.workspace.openTextDocument(reportPath).then((doc) => {
                            void vscode.window.showTextDocument(doc, { preview: false });
                        });
                    }
                });
            });
        })
    );

    // Command to generate a Markdown flow proposal (review artifact) via the CLI
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.flowPropose', async () => {
            const workspaceFolders = vscode.workspace.workspaceFolders;
            if (!workspaceFolders || workspaceFolders.length === 0) {
                void vscode.window.showErrorMessage('ToneGuard: No workspace open.');
                return;
            }
            const workspaceRoot = workspaceFolders[0].uri.fsPath;
            const reportPath = path.join(workspaceRoot, 'reports', 'flow-proposal.md');
            const args = [
                'flow',
                'propose',
                '--config',
                configPath,
                '--out',
                reportPath,
                workspaceRoot,
            ];

            outputChannel.show(true);
            outputChannel.appendLine(`ToneGuard: Generating flow proposal...`);
            outputChannel.appendLine(`ToneGuard: ${cliCommand} ${args.join(' ')}`);

            execFile(cliCommand, args, { cwd: workspaceRoot }, (error, stdout, stderr) => {
                if (stdout) {
                    outputChannel.appendLine(stdout);
                }
                if (stderr) {
                    outputChannel.appendLine(stderr);
                }
                if (error) {
                    const err = error as NodeJS.ErrnoException;
                    const message = err.message || String(err);
                    outputChannel.appendLine(`ToneGuard: Flow propose failed: ${message}`);
                    void vscode.window.showErrorMessage(
                        `ToneGuard flow propose failed. See output for details.`
                    );
                    return;
                }
                outputChannel.appendLine(
                    `ToneGuard: Flow proposal saved to ${reportPath}`
                );
                void vscode.window
                    .showInformationMessage(
                        'ToneGuard: Flow proposal generated.',
                        'Open Proposal'
                    )
                    .then((selection) => {
                        if (selection === 'Open Proposal') {
                            void vscode.workspace
                                .openTextDocument(reportPath)
                                .then((doc) => {
                                    void vscode.window.showTextDocument(doc, {
                                        preview: false,
                                    });
                                });
                        }
                    });
            });
        })
    );

    // Command to create a new flow spec file (artifact-first)
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.flowNew', async () => {
            const workspaceFolders = vscode.workspace.workspaceFolders;
            if (!workspaceFolders || workspaceFolders.length === 0) {
                void vscode.window.showErrorMessage('ToneGuard: No workspace open.');
                return;
            }
            const workspaceRoot = workspaceFolders[0].uri.fsPath;

            const name = await vscode.window.showInputBox({
                prompt: 'Flow name (human-readable)',
                placeHolder: 'E.g., "Create user account"',
            });
            if (!name) {
                return;
            }

            const entrypoint = await vscode.window.showInputBox({
                prompt: 'Entrypoint (route/command/job/etc)',
                placeHolder: 'E.g., "POST /users" or "dwg flow audit"',
            });
            if (!entrypoint) {
                return;
            }

            const language = await vscode.window.showInputBox({
                prompt: 'Language hint (optional)',
                placeHolder: 'rust / typescript / python',
            });

            function slugifyKebab(input: string): string {
                const trimmed = input.trim();
                let out = '';
                let prevDash = false;
                for (const ch of trimmed) {
                    if (/[A-Za-z0-9]/.test(ch)) {
                        out += ch.toLowerCase();
                        prevDash = false;
                    } else if (!prevDash && out.length > 0) {
                        out += '-';
                        prevDash = true;
                    }
                }
                return out.replace(/^-+|-+$/g, '');
            }

            const filename = `${slugifyKebab(name)}.md`;
            const outPath = path.join(workspaceRoot, 'flows', filename);

            let force = false;
            if (fs.existsSync(outPath)) {
                const choice = await vscode.window.showWarningMessage(
                    `Flow spec already exists: ${outPath}`,
                    'Overwrite',
                    'Cancel'
                );
                if (choice !== 'Overwrite') {
                    return;
                }
                force = true;
            }

            const args = [
                'flow',
                'new',
                '--config',
                configPath,
                '--name',
                name,
                '--entrypoint',
                entrypoint,
                '--out',
                outPath,
            ];
            if (language && language.trim().length > 0) {
                args.push('--language', language.trim());
            }
            if (force) {
                args.push('--force');
            }

            outputChannel.show(true);
            outputChannel.appendLine(`ToneGuard: Creating flow spec...`);
            outputChannel.appendLine(`ToneGuard: ${cliCommand} ${args.join(' ')}`);

            execFile(cliCommand, args, { cwd: workspaceRoot }, (error, stdout, stderr) => {
                if (stdout) {
                    outputChannel.appendLine(stdout);
                }
                if (stderr) {
                    outputChannel.appendLine(stderr);
                }
                if (error) {
                    const err = error as NodeJS.ErrnoException;
                    const message = err.message || String(err);
                    outputChannel.appendLine(`ToneGuard: Flow new failed: ${message}`);
                    void vscode.window.showErrorMessage(
                        `ToneGuard flow new failed. See output for details.`
                    );
                    return;
                }

                void vscode.workspace.openTextDocument(outPath).then((doc) => {
                    void vscode.window.showTextDocument(doc, { preview: false });
                });
            });
        })
    );
    
    // Command to show which server/config is being used
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.showInfo', () => {
            outputChannel.show();
            outputChannel.appendLine('---');
            outputChannel.appendLine(`Platform: ${getPlatformDir()}`);
            outputChannel.appendLine(`Server: ${command}`);
            outputChannel.appendLine(`CLI: ${cliCommand}`);
            outputChannel.appendLine(`Config: ${configPath}`);
            outputChannel.appendLine(`Bundled binary exists: ${getBundledServerPath(context) !== undefined}`);
            outputChannel.appendLine(`Bundled CLI exists: ${getBundledCliPath(context) !== undefined}`);
        })
    );

    // Command to install AI skill (auto-detect environment)
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkill', async () => {
            const env = await detectAIEnvironment();
            if (env) {
                await installSkill(context, env, outputChannel, 'writing');
            } else {
                // No environment detected, ask user which one to install for
                const choice = await vscode.window.showQuickPick(
                    [
                        { label: 'Claude Code', value: 'claude' as const },
                        { label: 'Codex', value: 'codex' as const },
                        { label: 'Both (Claude + Codex)', value: 'both' as const }
                    ],
                    { placeHolder: 'Select AI environment to install skill for' }
                );
                if (choice) {
                    if (choice.value === 'both') {
                        await installSkill(context, 'claude', outputChannel, 'writing');
                        await installSkill(context, 'codex', outputChannel, 'writing');
                    } else {
                        await installSkill(context, choice.value, outputChannel, 'writing');
                    }
                }
            }
        })
    );

    // Command to install logic flow guardrail skill (auto-detect environment)
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installLogicFlowSkill', async () => {
            const env = await detectAIEnvironment();
            if (env) {
                await installSkill(context, env, outputChannel, 'logic-flow');
            } else {
                const choice = await vscode.window.showQuickPick(
                    [
                        { label: 'Claude Code', value: 'claude' as const },
                        { label: 'Codex', value: 'codex' as const },
                        { label: 'Both (Claude + Codex)', value: 'both' as const }
                    ],
                    { placeHolder: 'Select AI environment to install logic flow skill for' }
                );
                if (choice) {
                    if (choice.value === 'both') {
                        await installSkill(context, 'claude', outputChannel, 'logic-flow');
                        await installSkill(context, 'codex', outputChannel, 'logic-flow');
                    } else {
                        await installSkill(context, choice.value, outputChannel, 'logic-flow');
                    }
                }
            }
        })
    );

    // Command to install skill for Claude Code specifically
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkillClaude', async () => {
            await installSkill(context, 'claude', outputChannel, 'writing');
        })
    );

    // Command to install skill for Codex specifically
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkillCodex', async () => {
            await installSkill(context, 'codex', outputChannel, 'writing');
        })
    );

    // Auto-detect AI environments and prompt for skill installation
    void (async () => {
        const envs = detectAIEnvironments();
        const detected: string[] = [];
        if (envs.cursor) detected.push('Cursor');
        if (envs.claude) detected.push('Claude Code');
        if (envs.codex) detected.push('Codex');
        
        if (detected.length > 0) {
            outputChannel.appendLine(`ToneGuard: Detected AI environments: ${detected.join(', ')}`);
            await promptSkillInstallationForAll(context, outputChannel);
        }
    })();
}

export async function deactivate(): Promise<void> {
    if (!client) {
        return;
    }
    await client.stop();
    client = undefined;
}
