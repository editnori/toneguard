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

type DefaultAIEditor = 'auto' | 'cursor' | 'claude-code' | 'codex';

// Binary availability status
type BinaryStatus = {
    available: boolean;
    path: string;
    mode: 'bundled' | 'PATH' | 'missing';
    platform: string;
    installCommand: string;
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
    cli: { mode: 'bundled' | 'PATH' | 'missing'; path: string };
    lsp: { mode: 'bundled' | 'PATH'; path: string };
    binaryAvailable: boolean;
    binaryInstallCommand: string;
    flowsCount: number | null;
    lastAudit: { when: string; findings: number } | null;
    lastProposal: { when: string } | null;
    detectedEnvs: DetectedEnvironments;
    defaultAIEditor: DefaultAIEditor;
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
            case 'copyInstall': {
                const binaryStatus = getBinaryStatus(this.context);
                await vscode.env.clipboard.writeText(binaryStatus.installCommand);
                void vscode.window.showInformationMessage(
                    'ToneGuard: Install command copied to clipboard.'
                );
                return;
            }
            case 'setDefaultEditor': {
                const value = typeof message.value === 'string' ? message.value : 'auto';
                const config = vscode.workspace.getConfiguration('dwg');
                await config.update(
                    'defaultAIEditor',
                    value,
                    vscode.ConfigurationTarget.Global
                );
                void vscode.window.showInformationMessage(
                    `ToneGuard: Default AI editor set to ${value}.`
                );
                await this.refresh();
                return;
            }
            case 'installSkill': {
                const envRaw = typeof message.env === 'string' ? message.env : '';
                const kindRaw = typeof message.kind === 'string' ? message.kind : '';
                const kind = kindRaw === 'logic-flow' ? 'logic-flow' : 'writing';
                if (envRaw === 'all') {
                    // Install all with progress indicator
                    await vscode.window.withProgress(
                        {
                            location: vscode.ProgressLocation.Notification,
                            title: `ToneGuard: Installing ${kind === 'logic-flow' ? 'Logic Flow' : 'Writing'} skills`,
                            cancellable: false,
                        },
                        async (progress) => {
                            const envs: AIEnvironment[] = ['cursor', 'claude', 'codex'];
                            const results: { env: string; success: boolean }[] = [];
                            
                            for (let i = 0; i < envs.length; i++) {
                                const env = envs[i];
                                const envName = getEnvDisplayName(env);
                                progress.report({
                                    increment: (100 / envs.length),
                                    message: `${envName}...`
                                });
                                
                                const success = await installSkill(this.context, env, this.outputChannel, kind);
                                results.push({ env: envName, success });
                            }
                            
                            // Show summary
                            const successCount = results.filter(r => r.success).length;
                            if (successCount === envs.length) {
                                void vscode.window.showInformationMessage(
                                    `ToneGuard: All ${kind} skills installed successfully!`
                                );
                            } else {
                                const failed = results.filter(r => !r.success).map(r => r.env).join(', ');
                                void vscode.window.showWarningMessage(
                                    `ToneGuard: Installed ${successCount}/${envs.length} skills. Failed: ${failed}`
                                );
                            }
                        }
                    );
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
        const binaryStatus = getBinaryStatus(this.context);
        const profileOptions =
            absConfigPath && fs.existsSync(absConfigPath)
                ? extractProfilesFromConfig(absConfigPath)
                : [];

        const config = vscode.workspace.getConfiguration('dwg');
        const strict = config.get<boolean>('strict', false);
        const noRepoChecks = config.get<boolean>('noRepoChecks', false);
        const defaultAIEditor = config.get<DefaultAIEditor>('defaultAIEditor', 'auto');

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
                mode: binaryStatus.mode,
                path: binaryStatus.available 
                    ? relOrBasename(workspaceRoot ?? '', binaryStatus.path) 
                    : `Not available for ${platform}`,
            },
            lsp: {
                mode:
                    bundledServer && this.serverCommand === bundledServer
                        ? 'bundled'
                        : 'PATH',
                path: relOrBasename(workspaceRoot ?? '', this.serverCommand),
            },
            binaryAvailable: binaryStatus.available,
            binaryInstallCommand: binaryStatus.installCommand,
            flowsCount,
            lastAudit,
            lastProposal,
            detectedEnvs,
            defaultAIEditor,
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

        /* Tab navigation */
        .tabs {
            display: flex;
            gap: 0;
            border-bottom: 1px solid var(--tg-border);
            margin-bottom: 12px;
            overflow-x: auto;
        }

        .tab {
            padding: 8px 14px;
            font-size: 12px;
            background: transparent;
            border: none;
            border-bottom: 2px solid transparent;
            color: var(--tg-muted);
            cursor: pointer;
            white-space: nowrap;
            transition: color 0.15s, border-color 0.15s;
        }

        .tab:hover {
            color: var(--tg-text);
        }

        .tab.active {
            color: var(--tg-accent);
            border-bottom-color: var(--tg-accent);
        }

        .tab-content {
            display: none;
        }

        .tab-content.active {
            display: block;
        }

        .alert {
            padding: 10px 12px;
            border-radius: 8px;
            font-size: 12px;
            margin-bottom: 10px;
        }

        .alert.warn {
            background: rgba(255, 180, 0, 0.1);
            border: 1px solid var(--tg-warn);
            color: var(--tg-warn);
        }

        .alert.info {
            background: rgba(0, 150, 255, 0.1);
            border: 1px solid var(--tg-accent);
            color: var(--tg-accent);
        }
    </style>
</head>
<body>
    <div class="container">
        <!-- Quick Actions - Always visible -->
        <div class="card actions">
            <button data-action="runRecommended">▶ Run ToneGuard</button>
            <div class="grid-2">
                <button class="secondary" data-action="runAudit">Flow Audit</button>
                <button class="secondary" data-action="runProposal">Proposal</button>
            </div>
        </div>

        <!-- Tab Navigation -->
        <div class="tabs">
            <button class="tab active" data-tab="findings">Findings</button>
            <button class="tab" data-tab="config">Config</button>
            <button class="tab" data-tab="skills">Skills</button>
            <button class="tab" data-tab="status">Status</button>
        </div>

        <!-- Tab: Findings -->
        <div id="tab-findings" class="tab-content active">
            <div id="binaryAlert" class="alert warn" style="display: none;">
                CLI not available. <a href="#" data-action="copyInstall">Copy install command</a>
            </div>
            <div id="findingsSummary" class="card">
                <div class="card-row">
                    <span class="label">Last audit</span>
                    <span class="value" id="lastAudit"></span>
                </div>
                <div class="card-row">
                    <span class="label">Last proposal</span>
                    <span class="value" id="lastProposal"></span>
                </div>
            </div>
            <button class="ghost" data-action="newFlow">+ New Flow Spec</button>
        </div>

        <!-- Tab: Config -->
        <div id="tab-config" class="tab-content">
            <div>
                <div class="section-title">Categories</div>
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
                <div class="section-title">Profiles</div>
                <div class="card">
                    <div class="card-row">
                        <span class="label">Active</span>
                        <span class="value" id="currentProfile"></span>
                    </div>
                    <div id="profileOptions" class="input-row" style="flex-wrap: wrap;"></div>
                    <button class="ghost" data-action="clearProfile">Clear Profile</button>
                </div>
            </div>
            <div>
                <div class="section-title">Quick Settings</div>
                <div class="card">
                    <label class="toggle">
                        <input id="strictToggle" type="checkbox" />
                        Strict mode
                    </label>
                    <label class="toggle">
                        <input id="repoToggle" type="checkbox" />
                        Skip repo-wide checks
                    </label>
                </div>
            </div>
            <div class="card">
                <button class="secondary" data-action="openConfig">Open Config File</button>
            </div>
        </div>

        <!-- Tab: Skills -->
        <div id="tab-skills" class="tab-content">
            <div>
                <div class="section-title">AI Editor</div>
                <div class="card">
                    <div class="card-row">
                        <span class="label">Default</span>
                        <select id="defaultAIEditor" class="value">
                            <option value="auto">Auto-detect</option>
                            <option value="cursor">Cursor</option>
                            <option value="claude-code">Claude Code</option>
                            <option value="codex">Codex</option>
                        </select>
                    </div>
                </div>
            </div>
            <div>
                <div class="section-title">Writing Style</div>
                <div class="card">
                    <div class="card-row" data-skill="writing" data-env="cursor">
                        <span class="label">Cursor</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="writing" data-env="cursor">Install</button>
                        </div>
                    </div>
                    <div class="card-row" data-skill="writing" data-env="claude">
                        <span class="label">Claude</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="writing" data-env="claude">Install</button>
                        </div>
                    </div>
                    <div class="card-row" data-skill="writing" data-env="codex">
                        <span class="label">Codex</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="writing" data-env="codex">Install</button>
                        </div>
                    </div>
                    <button class="ghost" data-action="installSkill" data-kind="writing" data-env="all">Install All</button>
                </div>
            </div>
            <div>
                <div class="section-title">Logic Flow Guardrails</div>
                <div class="card">
                    <div class="card-row" data-skill="logic-flow" data-env="cursor">
                        <span class="label">Cursor</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="logic-flow" data-env="cursor">Install</button>
                        </div>
                    </div>
                    <div class="card-row" data-skill="logic-flow" data-env="claude">
                        <span class="label">Claude</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="logic-flow" data-env="claude">Install</button>
                        </div>
                    </div>
                    <div class="card-row" data-skill="logic-flow" data-env="codex">
                        <span class="label">Codex</span>
                        <div class="input-row">
                            <span class="badge" data-skill-badge>—</span>
                            <button data-action="installSkill" data-kind="logic-flow" data-env="codex">Install</button>
                        </div>
                    </div>
                    <button class="ghost" data-action="installSkill" data-kind="logic-flow" data-env="all">Install All</button>
                </div>
            </div>
        </div>

        <!-- Tab: Status -->
        <div id="tab-status" class="tab-content">
            <div class="section-title">Environment</div>
            <div class="card">
                <div class="card-row">
                    <span class="label">Platform</span>
                    <span class="value" id="platform"></span>
                </div>
                <div class="card-row">
                    <span class="label">CLI</span>
                    <span class="value" id="cliPath"></span>
                </div>
                <div class="card-row">
                    <span class="label">LSP</span>
                    <span class="value" id="lspPath"></span>
                </div>
            </div>
            <div class="section-title">Config</div>
            <div class="card">
                <div class="card-row">
                    <span class="label">Path</span>
                    <span class="value" id="configPath"></span>
                </div>
                <div class="card-row">
                    <span class="label">Source</span>
                    <span class="badge" id="configSource"></span>
                </div>
                <div class="card-row">
                    <span class="label">Flows</span>
                    <span class="value" id="flowsCount"></span>
                </div>
            </div>
            <div class="section-title">Detected AI Environments</div>
            <div class="card">
                <div class="card-row">
                    <span class="label">Cursor</span>
                    <span class="badge" id="envCursor">—</span>
                </div>
                <div class="card-row">
                    <span class="label">Claude Code</span>
                    <span class="badge" id="envClaude">—</span>
                </div>
                <div class="card-row">
                    <span class="label">Codex</span>
                    <span class="badge" id="envCodex">—</span>
                </div>
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

        // Tab switching
        document.querySelectorAll('.tab').forEach((tab) => {
            tab.addEventListener('click', () => {
                const tabName = tab.dataset.tab;
                if (!tabName) return;
                
                // Update active tab button
                document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
                tab.classList.add('active');
                
                // Update active tab content
                document.querySelectorAll('.tab-content').forEach(content => {
                    content.classList.toggle('active', content.id === 'tab-' + tabName);
                });
            });
        });

        // Show binary alert if not available
        if (!state.binaryAvailable) {
            const alert = document.getElementById('binaryAlert');
            if (alert) {
                alert.style.display = 'block';
                alert.querySelector('[data-action="copyInstall"]').addEventListener('click', (e) => {
                    e.preventDefault();
                    vscode.postMessage({ type: 'copyInstall' });
                });
            }
        }

        // AI Editor selector
        const editorSelect = document.getElementById('defaultAIEditor');
        if (editorSelect) {
            editorSelect.value = state.defaultAIEditor || 'auto';
            editorSelect.addEventListener('change', () => {
                vscode.postMessage({ type: 'setDefaultEditor', value: editorSelect.value });
            });
        }

        // Detected environments
        function setBadge(id, detected) {
            const el = document.getElementById(id);
            if (el) {
                el.textContent = detected ? 'Yes' : 'No';
                el.classList.toggle('ok', detected);
            }
        }
        if (state.detectedEnvs) {
            setBadge('envCursor', state.detectedEnvs.cursor);
            setBadge('envClaude', state.detectedEnvs.claude);
            setBadge('envCodex', state.detectedEnvs.codex);
        }

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
 * Gets the default AI editor from settings, or auto-detects.
 */
function getDefaultAIEditor(): DefaultAIEditor {
    const config = vscode.workspace.getConfiguration('dwg');
    return config.get<DefaultAIEditor>('defaultAIEditor', 'auto');
}

/**
 * Resolves which AI editor to use based on settings and availability.
 */
function resolveAIEditor(envs: DetectedEnvironments): AIEnvironment | null {
    const defaultEditor = getDefaultAIEditor();
    
    if (defaultEditor !== 'auto') {
        // User has a preference - check if it's available
        if (defaultEditor === 'cursor' && envs.cursor) return 'cursor';
        if (defaultEditor === 'claude-code' && envs.claude) return 'claude';
        if (defaultEditor === 'codex' && envs.codex) return 'codex';
        // Fall through to auto if preferred editor not available
    }
    
    // Auto-detect: prioritize based on likely context
    if (envs.cursor) return 'cursor';
    if (envs.claude) return 'claude';
    if (envs.codex) return 'codex';
    return null;
}

/**
 * Fix findings using an AI agent (Cursor Composer, Claude, or Codex).
 * For Cursor: Attempts to use native chat API, falls back to clipboard.
 * For Claude/Codex: Copies to clipboard with instructions.
 */
async function fixWithAI(
    context: vscode.ExtensionContext,
    findings: any[],
    env: AIEnvironment,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const prompt = generateFixPrompt(findings);
    
    outputChannel.appendLine(`ToneGuard: Preparing ${findings.length} fix instructions for ${env}`);
    
    if (env === 'cursor') {
        // Try multiple Cursor-specific commands to insert the prompt
        const cursorCommands = [
            'cursor.chat.newWithPrompt',      // New chat with prompt (if available)
            'cursor.composer.newWithPrompt',  // New composer with prompt
            'aichat.newchatwithprompt',       // Alternative chat command
            'workbench.action.chat.open',     // Generic VS Code chat
        ];
        
        let inserted = false;
        for (const cmd of cursorCommands) {
            try {
                // Try commands that accept a prompt argument
                await vscode.commands.executeCommand(cmd, { prompt, query: prompt, text: prompt });
                inserted = true;
                void vscode.window.showInformationMessage(
                    `ToneGuard: ${findings.length} fixes sent to Cursor chat.`
                );
                break;
            } catch {
                // Command doesn't exist or doesn't accept this format, try next
            }
        }
        
        if (!inserted) {
            // Fallback: Copy to clipboard and open composer, try to auto-paste
            await vscode.env.clipboard.writeText(prompt);
            try {
                await vscode.commands.executeCommand('cursor.composer.new');
                // Wait for composer to be ready, then try to paste
                await new Promise(resolve => setTimeout(resolve, 300));
                try {
                    await vscode.commands.executeCommand('editor.action.clipboardPasteAction');
                    void vscode.window.showInformationMessage(
                        `ToneGuard: ${findings.length} fix instructions pasted into Composer.`
                    );
                } catch {
                    void vscode.window.showInformationMessage(
                        `ToneGuard: ${findings.length} fix instructions copied. Press Cmd+V to paste.`
                    );
                }
            } catch {
                // Can't even open composer, just inform user
                void vscode.window.showInformationMessage(
                    `ToneGuard: ${findings.length} fix instructions copied. Open Composer (Cmd+I) and paste.`
                );
            }
        }
        return;
    }
    
    // For Claude/Codex: Copy to clipboard and try to focus their terminal
    await vscode.env.clipboard.writeText(prompt);
    outputChannel.appendLine(`ToneGuard: Copied ${findings.length} fix instructions to clipboard`);
    
    if (env === 'claude') {
        // Try to focus Claude terminal if it exists
        const terminals = vscode.window.terminals;
        const claudeTerminal = terminals.find(t => 
            t.name.toLowerCase().includes('claude') || 
            t.name.toLowerCase().includes('anthropic')
        );
        if (claudeTerminal) {
            claudeTerminal.show();
            void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied. Press Cmd+V in Claude terminal.`
            );
        } else {
            void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied. Paste in Claude Code to apply.`
            );
        }
    } else {
        // For Codex - try to focus the terminal
        const terminals = vscode.window.terminals;
        const codexTerminal = terminals.find(t => 
            t.name.toLowerCase().includes('codex') || 
            t.name.toLowerCase().includes('openai')
        );
        if (codexTerminal) {
            codexTerminal.show();
            void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied. Press Cmd+V in Codex terminal.`
            );
        } else {
            void vscode.window.showInformationMessage(
                `ToneGuard: ${findings.length} fix instructions copied. Paste in ${getEnvDisplayName(env)} to apply.`
            );
        }
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
 * Checks if a command exists in PATH.
 */
function commandExistsInPath(command: string): boolean {
    try {
        const { execSync } = require('child_process');
        const which = os.platform() === 'win32' ? 'where' : 'which';
        execSync(`${which} ${command}`, { stdio: 'ignore' });
        return true;
    } catch {
        return false;
    }
}

/**
 * Gets the binary availability status with helpful installation instructions.
 */
function getBinaryStatus(context: vscode.ExtensionContext): BinaryStatus {
    const platform = getPlatformDir();
    const bundledPath = getBundledCliPath(context);
    
    if (bundledPath) {
        return {
            available: true,
            path: bundledPath,
            mode: 'bundled',
            platform,
            installCommand: '',
        };
    }
    
    // Check if dwg is in PATH
    if (commandExistsInPath('dwg')) {
        return {
            available: true,
            path: 'dwg',
            mode: 'PATH',
            platform,
            installCommand: '',
        };
    }
    
    // Binary not available - provide install instructions based on platform
    let installCommand: string;
    if (platform.startsWith('darwin')) {
        installCommand = 'brew install toneguard  # or: cargo install --git https://github.com/editnori/toneguard dwg-cli';
    } else if (platform.startsWith('win32')) {
        installCommand = 'cargo install --git https://github.com/editnori/toneguard dwg-cli';
    } else {
        installCommand = 'cargo install --git https://github.com/editnori/toneguard dwg-cli';
    }
    
    return {
        available: false,
        path: '',
        mode: 'missing',
        platform,
        installCommand,
    };
}

/**
 * Shows an error message when binaries are not available for the platform.
 */
async function showBinaryMissingError(status: BinaryStatus): Promise<void> {
    const platformNames: Record<string, string> = {
        'darwin-arm64': 'macOS (Apple Silicon)',
        'darwin-x64': 'macOS (Intel)',
        'win32-x64': 'Windows',
        'linux-x64': 'Linux (x64)',
        'linux-arm64': 'Linux (ARM64)',
    };
    
    const platformName = platformNames[status.platform] || status.platform;
    
    const choice = await vscode.window.showErrorMessage(
        `ToneGuard: Binaries not available for ${platformName}. Install manually to use flow analysis features.`,
        'Copy Install Command',
        'Open GitHub',
        'Dismiss'
    );
    
    if (choice === 'Copy Install Command') {
        await vscode.env.clipboard.writeText(status.installCommand);
        void vscode.window.showInformationMessage(
            'Install command copied to clipboard. Run it in your terminal.'
        );
    } else if (choice === 'Open GitHub') {
        void vscode.env.openExternal(
            vscode.Uri.parse('https://github.com/editnori/toneguard#installation')
        );
    }
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
 * Uses multiple detection methods for reliability:
 * - Cursor: appName, uriScheme, or CURSOR_* env vars
 * - Claude Code: ~/.claude directory, CLAUDE_* env vars, or workspace .claude
 * - Codex: ~/.codex directory, CODEX_* env vars, or workspace .codex
 */
function detectAIEnvironments(): DetectedEnvironments {
    // Cursor detection: check appName, uriScheme, and environment
    const isCursor = 
        vscode.env.appName.toLowerCase().includes('cursor') ||
        vscode.env.uriScheme === 'cursor' ||
        !!process.env.CURSOR_CHANNEL ||
        !!process.env.CURSOR_TRACE;
    
    const homeDir = os.homedir();
    const workspaceFolders = vscode.workspace.workspaceFolders;
    const workspaceRoot = workspaceFolders?.[0]?.uri.fsPath;
    
    // Claude Code detection: global config, env vars, or workspace marker
    const hasClaude = 
        fs.existsSync(path.join(homeDir, '.claude')) ||
        !!process.env.CLAUDE_API_KEY ||
        !!process.env.ANTHROPIC_API_KEY ||
        (workspaceRoot ? fs.existsSync(path.join(workspaceRoot, '.claude')) : false);
    
    // Codex detection: global config, env vars, or workspace marker
    const hasCodex = 
        fs.existsSync(path.join(homeDir, '.codex')) ||
        !!process.env.CODEX_HOME ||
        !!process.env.OPENAI_API_KEY ||
        (workspaceRoot ? fs.existsSync(path.join(workspaceRoot, '.codex')) : false);

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

// Flow Map Panel for visualizing control flow graphs
class FlowMapPanel {
    public static currentPanel: FlowMapPanel | undefined;
    public static readonly viewType = 'toneguard.flowMap';

    private readonly panel: vscode.WebviewPanel;
    private readonly extensionUri: vscode.Uri;
    private disposables: vscode.Disposable[] = [];
    private cliPath: string;
    private configPath: string;
    private outputChannel: vscode.OutputChannel;

    public static createOrShow(
        extensionUri: vscode.Uri,
        cliPath: string,
        configPath: string,
        outputChannel: vscode.OutputChannel,
        filePath?: string
    ): FlowMapPanel {
        const column = vscode.window.activeTextEditor
            ? vscode.window.activeTextEditor.viewColumn
            : undefined;

        // If we already have a panel, show it
        if (FlowMapPanel.currentPanel) {
            FlowMapPanel.currentPanel.panel.reveal(column);
            if (filePath) {
                FlowMapPanel.currentPanel.loadFile(filePath);
            }
            return FlowMapPanel.currentPanel;
        }

        // Otherwise, create a new panel
        const panel = vscode.window.createWebviewPanel(
            FlowMapPanel.viewType,
            'ToneGuard Flow Map',
            column || vscode.ViewColumn.One,
            {
                enableScripts: true,
                retainContextWhenHidden: true,
            }
        );

        FlowMapPanel.currentPanel = new FlowMapPanel(
            panel,
            extensionUri,
            cliPath,
            configPath,
            outputChannel
        );

        if (filePath) {
            FlowMapPanel.currentPanel.loadFile(filePath);
        }

        return FlowMapPanel.currentPanel;
    }

    private constructor(
        panel: vscode.WebviewPanel,
        extensionUri: vscode.Uri,
        cliPath: string,
        configPath: string,
        outputChannel: vscode.OutputChannel
    ) {
        this.panel = panel;
        this.extensionUri = extensionUri;
        this.cliPath = cliPath;
        this.configPath = configPath;
        this.outputChannel = outputChannel;

        this.update();

        this.panel.onDidDispose(() => this.dispose(), null, this.disposables);

        this.panel.webview.onDidReceiveMessage(
            async (message: any) => {
                switch (message.command) {
                    case 'selectFile':
                        const files = await vscode.window.showOpenDialog({
                            canSelectFiles: true,
                            canSelectFolders: false,
                            canSelectMany: false,
                            filters: {
                                'Source Files': ['rs', 'ts', 'tsx', 'js', 'jsx', 'py'],
                            },
                        });
                        if (files && files[0]) {
                            this.loadFile(files[0].fsPath);
                        }
                        break;

                    case 'loadFunction':
                        this.loadFunction(message.file, message.functionName);
                        break;

                    case 'openFile':
                        const uri = vscode.Uri.file(message.file);
                        const doc = await vscode.workspace.openTextDocument(uri);
                        await vscode.window.showTextDocument(doc, {
                            selection: message.line
                                ? new vscode.Range(message.line - 1, 0, message.line - 1, 0)
                                : undefined,
                        });
                        break;
                }
            },
            null,
            this.disposables
        );
    }

    public loadFile(filePath: string): void {
        this.runCliGraph(filePath).then((result) => {
            this.panel.webview.postMessage({
                type: 'graphData',
                data: result,
                filePath,
            });
        }).catch((err) => {
            this.panel.webview.postMessage({
                type: 'error',
                message: err.message,
            });
        });
    }

    public loadFunction(filePath: string, functionName: string): void {
        this.runCliGraph(filePath, functionName).then((result) => {
            this.panel.webview.postMessage({
                type: 'graphData',
                data: result,
                filePath,
                functionName,
            });
        }).catch((err) => {
            this.panel.webview.postMessage({
                type: 'error',
                message: err.message,
            });
        });
    }

    private async runCliGraph(
        filePath: string,
        functionName?: string
    ): Promise<any> {
        return new Promise((resolve, reject) => {
            const args = ['flow', 'graph', '--file', filePath, '--with-logic'];
            if (functionName) {
                args.push('--fn', functionName);
            }

            this.outputChannel.appendLine(`ToneGuard: Running ${this.cliPath} ${args.join(' ')}`);

            execFile(this.cliPath, args, { maxBuffer: 10 * 1024 * 1024 }, (error, stdout, stderr) => {
                if (error) {
                    this.outputChannel.appendLine(`ToneGuard: Flow graph error: ${stderr || error.message}`);
                    reject(new Error(stderr || error.message));
                    return;
                }

                try {
                    const data = JSON.parse(stdout);
                    resolve(data);
                } catch (e) {
                    reject(new Error('Failed to parse CFG output'));
                }
            });
        });
    }

    private update(): void {
        const webview = this.panel.webview;
        this.panel.title = 'ToneGuard Flow Map';
        this.panel.webview.html = this.getHtmlForWebview(webview);
    }

    private getHtmlForWebview(webview: vscode.Webview): string {
        const nonce = getNonce();

        return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
    <title>ToneGuard Flow Map</title>
    <style>
        :root {
            --bg: var(--vscode-editor-background);
            --fg: var(--vscode-editor-foreground);
            --border: var(--vscode-panel-border);
            --accent: var(--vscode-button-background);
            --btn-fg: var(--vscode-button-foreground);
            --input-bg: var(--vscode-input-background);
            --input-border: var(--vscode-input-border);
        }
        * { box-sizing: border-box; }
        body {
            margin: 0;
            padding: 16px;
            font-family: var(--vscode-font-family);
            background: var(--bg);
            color: var(--fg);
        }
        h1 { margin: 0 0 16px 0; font-size: 1.5em; }
        .toolbar {
            display: flex;
            gap: 12px;
            margin-bottom: 16px;
            align-items: center;
            flex-wrap: wrap;
        }
        button {
            background: var(--accent);
            color: var(--btn-fg);
            border: none;
            padding: 8px 16px;
            cursor: pointer;
            border-radius: 4px;
            font-size: 13px;
        }
        button:hover { opacity: 0.9; }
        button:disabled { opacity: 0.5; cursor: not-allowed; }
        select {
            background: var(--input-bg);
            color: var(--fg);
            border: 1px solid var(--input-border);
            padding: 8px;
            border-radius: 4px;
            min-width: 200px;
        }
        .file-path {
            font-size: 12px;
            color: var(--vscode-descriptionForeground);
            margin-left: auto;
        }
        .container {
            display: grid;
            grid-template-columns: 280px 1fr;
            gap: 16px;
            height: calc(100vh - 140px);
        }
        .sidebar {
            background: var(--vscode-sideBar-background);
            border: 1px solid var(--border);
            border-radius: 4px;
            overflow: auto;
            padding: 12px;
        }
        .sidebar h3 {
            margin: 0 0 12px 0;
            font-size: 13px;
            text-transform: uppercase;
            color: var(--vscode-descriptionForeground);
        }
        .function-list {
            list-style: none;
            padding: 0;
            margin: 0;
        }
        .function-list li {
            padding: 8px;
            cursor: pointer;
            border-radius: 4px;
            margin-bottom: 4px;
            font-size: 13px;
        }
        .function-list li:hover {
            background: var(--vscode-list-hoverBackground);
        }
        .function-list li.selected {
            background: var(--vscode-list-activeSelectionBackground);
            color: var(--vscode-list-activeSelectionForeground);
        }
        .function-stats {
            font-size: 11px;
            color: var(--vscode-descriptionForeground);
        }
        .main-panel {
            background: var(--vscode-sideBar-background);
            border: 1px solid var(--border);
            border-radius: 4px;
            overflow: auto;
            display: flex;
            flex-direction: column;
        }
        .graph-container {
            flex: 1;
            padding: 16px;
            overflow: auto;
        }
        pre.mermaid-code {
            background: var(--vscode-textBlockQuote-background);
            padding: 12px;
            border-radius: 4px;
            overflow: auto;
            font-size: 12px;
            white-space: pre-wrap;
            margin: 0;
        }
        .findings-panel {
            border-top: 1px solid var(--border);
            padding: 12px;
            max-height: 200px;
            overflow: auto;
        }
        .findings-panel h4 {
            margin: 0 0 8px 0;
            font-size: 12px;
            color: var(--vscode-descriptionForeground);
        }
        .finding-item {
            padding: 8px;
            margin-bottom: 4px;
            border-radius: 4px;
            font-size: 12px;
            cursor: pointer;
        }
        .finding-item:hover {
            background: var(--vscode-list-hoverBackground);
        }
        .finding-item.warning {
            border-left: 3px solid var(--vscode-editorWarning-foreground);
        }
        .finding-item.error {
            border-left: 3px solid var(--vscode-editorError-foreground);
        }
        .finding-item.info {
            border-left: 3px solid var(--vscode-editorInfo-foreground);
        }
        .empty-state {
            text-align: center;
            padding: 60px 20px;
            color: var(--vscode-descriptionForeground);
        }
        .empty-state h2 { margin: 0 0 12px 0; }
        .loading {
            text-align: center;
            padding: 40px;
            color: var(--vscode-descriptionForeground);
        }
        .error-msg {
            background: var(--vscode-inputValidation-errorBackground);
            border: 1px solid var(--vscode-inputValidation-errorBorder);
            padding: 12px;
            border-radius: 4px;
            margin: 16px;
        }
    </style>
</head>
<body>
    <h1>🔀 Flow Map</h1>
    <div class="toolbar">
        <button id="selectFileBtn">📂 Select File</button>
        <select id="functionSelect" disabled>
            <option value="">-- Select a function --</option>
        </select>
        <span class="file-path" id="filePath"></span>
    </div>

    <div class="container">
        <div class="sidebar">
            <h3>Functions</h3>
            <ul class="function-list" id="functionList">
                <li class="empty-state">Select a file to see functions</li>
            </ul>
        </div>
        <div class="main-panel">
            <div class="graph-container" id="graphContainer">
                <div class="empty-state">
                    <h2>Control Flow Graph Visualizer</h2>
                    <p>Select a source file (.rs, .ts, .js, .py) to visualize its control flow graph.</p>
                    <p>Click on a function to see its CFG as a Mermaid diagram.</p>
                </div>
            </div>
            <div class="findings-panel" id="findingsPanel" style="display: none;">
                <h4>Logic Findings</h4>
                <div id="findingsList"></div>
            </div>
        </div>
    </div>

    <script nonce="${nonce}">
        const vscode = acquireVsCodeApi();
        let currentData = null;
        let currentFile = '';

        document.getElementById('selectFileBtn').addEventListener('click', () => {
            vscode.postMessage({ command: 'selectFile' });
        });

        document.getElementById('functionSelect').addEventListener('change', (e) => {
            const fn = e.target.value;
            if (fn && currentFile) {
                vscode.postMessage({ command: 'loadFunction', file: currentFile, functionName: fn });
            }
        });

        window.addEventListener('message', event => {
            const message = event.data;
            
            if (message.type === 'graphData') {
                currentData = message.data;
                currentFile = message.filePath || '';
                document.getElementById('filePath').textContent = currentFile;
                renderFunctionList(message.data.cfgs || []);
                
                // If a single function was loaded, show its graph
                if (message.functionName && message.data.cfgs?.length > 0) {
                    const cfg = message.data.cfgs[0];
                    renderGraph(cfg);
                    highlightFunction(cfg.name);
                } else if (message.data.cfgs?.length > 0) {
                    // Show first function by default
                    const cfg = message.data.cfgs[0];
                    renderGraph(cfg);
                    highlightFunction(cfg.name);
                }
                
                // Show findings if any
                if (message.data.logic_findings?.length > 0) {
                    renderFindings(message.data.logic_findings);
                } else {
                    document.getElementById('findingsPanel').style.display = 'none';
                }
            }
            
            if (message.type === 'error') {
                document.getElementById('graphContainer').innerHTML = 
                    '<div class="error-msg">' + escapeHtml(message.message) + '</div>';
            }
        });

        function renderFunctionList(cfgs) {
            const list = document.getElementById('functionList');
            const select = document.getElementById('functionSelect');
            
            if (!cfgs || cfgs.length === 0) {
                list.innerHTML = '<li class="empty-state">No functions found</li>';
                select.innerHTML = '<option value="">-- No functions --</option>';
                select.disabled = true;
                return;
            }

            list.innerHTML = cfgs.map(cfg => 
                '<li data-fn="' + escapeHtml(cfg.name) + '">' +
                '<strong>' + escapeHtml(cfg.name) + '</strong>' +
                '<div class="function-stats">' +
                cfg.nodes + ' nodes, ' + cfg.edges + ' edges, ' +
                (cfg.unreachable > 0 ? '<span style="color: var(--vscode-editorWarning-foreground)">' + cfg.unreachable + ' unreachable</span>' : '0 unreachable') +
                '</div></li>'
            ).join('');

            select.innerHTML = '<option value="">-- Select function --</option>' +
                cfgs.map(cfg => '<option value="' + escapeHtml(cfg.name) + '">' + escapeHtml(cfg.name) + '</option>').join('');
            select.disabled = false;

            // Add click handlers
            list.querySelectorAll('li[data-fn]').forEach(li => {
                li.addEventListener('click', () => {
                    const fn = li.getAttribute('data-fn');
                    const cfg = cfgs.find(c => c.name === fn);
                    if (cfg) {
                        renderGraph(cfg);
                        highlightFunction(fn);
                        select.value = fn;
                    }
                });
            });
        }

        function highlightFunction(name) {
            document.querySelectorAll('.function-list li').forEach(li => {
                li.classList.toggle('selected', li.getAttribute('data-fn') === name);
            });
        }

        function renderGraph(cfg) {
            const container = document.getElementById('graphContainer');
            
            if (cfg.mermaid) {
                container.innerHTML = 
                    '<h3>' + escapeHtml(cfg.name) + ' (line ' + cfg.start_line + ')</h3>' +
                    '<p>Language: ' + cfg.language + ' | ' + cfg.nodes + ' nodes | ' + cfg.edges + ' edges | ' + cfg.exits.length + ' exits</p>' +
                    '<pre class="mermaid-code">' + escapeHtml(cfg.mermaid) + '</pre>' +
                    '<p style="font-size: 11px; color: var(--vscode-descriptionForeground);">Copy the Mermaid code above and paste it in a Mermaid-compatible viewer to see the diagram.</p>';
            } else {
                // Generate a simple Mermaid representation from stats
                container.innerHTML = 
                    '<h3>' + escapeHtml(cfg.name) + ' (line ' + cfg.start_line + ')</h3>' +
                    '<p>Language: ' + cfg.language + '</p>' +
                    '<p>' + cfg.nodes + ' nodes, ' + cfg.edges + ' edges, ' + cfg.exits.length + ' exit points</p>' +
                    (cfg.unreachable > 0 ? '<p style="color: var(--vscode-editorWarning-foreground)">⚠️ ' + cfg.unreachable + ' unreachable nodes detected</p>' : '') +
                    '<p>Exit node IDs: ' + cfg.exits.join(', ') + '</p>';
            }
        }

        function renderFindings(findings) {
            const panel = document.getElementById('findingsPanel');
            const list = document.getElementById('findingsList');
            
            panel.style.display = 'block';
            list.innerHTML = findings.map(f => 
                '<div class="finding-item ' + (f.severity || 'info') + '" data-path="' + escapeHtml(f.path) + '" data-line="' + (f.line || '') + '">' +
                '<strong>[' + (f.category || 'finding') + ']</strong> ' + escapeHtml(f.message) +
                (f.line ? ' (line ' + f.line + ')' : '') +
                '</div>'
            ).join('');

            list.querySelectorAll('.finding-item').forEach(item => {
                item.addEventListener('click', () => {
                    const path = item.getAttribute('data-path');
                    const line = parseInt(item.getAttribute('data-line'), 10) || undefined;
                    if (path) {
                        vscode.postMessage({ command: 'openFile', file: path, line });
                    }
                });
            });
        }

        function escapeHtml(text) {
            if (!text) return '';
            const div = document.createElement('div');
            div.textContent = text;
            return div.innerHTML;
        }
    </script>
</body>
</html>`;
    }

    public dispose(): void {
        FlowMapPanel.currentPanel = undefined;
        this.panel.dispose();
        while (this.disposables.length) {
            const x = this.disposables.pop();
            if (x) {
                x.dispose();
            }
        }
    }
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

            // Pre-flight check: ensure binary is available
            const binaryStatus = getBinaryStatus(context);
            if (!binaryStatus.available) {
                outputChannel.show(true);
                outputChannel.appendLine(`ToneGuard: Binary not available for platform: ${binaryStatus.platform}`);
                outputChannel.appendLine(`ToneGuard: Install command: ${binaryStatus.installCommand}`);
                await showBinaryMissingError(binaryStatus);
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

            // Create reports directory if needed
            const reportsDir = path.join(workspaceRoot, 'reports');
            if (!fs.existsSync(reportsDir)) {
                fs.mkdirSync(reportsDir, { recursive: true });
            }
            const markdownLintPath = path.join(reportsDir, 'markdown-lint.json');

            try {
                await vscode.window.withProgress(
                    {
                        location: vscode.ProgressLocation.Notification,
                        title: 'ToneGuard: Running recommended review',
                        cancellable: false,
                    },
                    async (progress) => {
                        // Step 1: Lint markdown/prose files
                        progress.report({ message: 'Scanning markdown files…' });
                        try {
                            await runCli(
                                [
                                    '--config',
                                    configPath,
                                    '--json',
                                    '--no-repo-checks',
                                    workspaceRoot,
                                ],
                                'Scanning markdown files…'
                            );
                        } catch {
                            // Markdown lint may exit non-zero if issues found, that's OK
                            outputChannel.appendLine('ToneGuard: Markdown scan complete (issues may exist)');
                        }

                        // Step 2: Run code flow audit  
                        progress.report({ message: 'Running flow audit…' });
                        const flowsDir = path.join(workspaceRoot, 'flows');
                        await runCli(
                            [
                                'flow',
                                'audit',
                                '--config',
                                configPath,
                                '--flows',
                                flowsDir,
                                '--out',
                                auditPath,
                                workspaceRoot,
                            ],
                            'Running flow audit…'
                        );

                        // Step 3: Generate proposal
                        progress.report({ message: 'Generating flow proposal…' });
                        await runCli(
                            [
                                'flow',
                                'propose',
                                '--config',
                                configPath,
                                '--flows',
                                flowsDir,
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
                        'ToneGuard: All clear! No issues found.'
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
                        // Open composer and try to auto-paste
                        await vscode.commands.executeCommand('cursor.composer.new');
                        // Wait for composer to be ready, then paste
                        await new Promise(resolve => setTimeout(resolve, 300));
                        try {
                            await vscode.commands.executeCommand('editor.action.clipboardPasteAction');
                            void vscode.window.showInformationMessage(
                                `ToneGuard: ${findingCount} fix instructions pasted into Composer.`
                            );
                        } catch {
                            // Paste failed, user needs to do it manually
                            void vscode.window.showInformationMessage(
                                'ToneGuard: Fix instructions copied. Press Cmd+V to paste in Composer.'
                            );
                        }
                    } catch {
                        void vscode.window.showInformationMessage(
                            'ToneGuard: Fix instructions copied. Open Composer (Cmd+I) and paste.'
                        );
                    }
                } else if (choice === 'Fix with Claude') {
                    const prompt = generateFixPrompt(findings);
                    await vscode.env.clipboard.writeText(prompt);
                    // Try to focus Claude Code terminal if it exists
                    const terminals = vscode.window.terminals;
                    const claudeTerminal = terminals.find(t => 
                        t.name.toLowerCase().includes('claude') || 
                        t.name.toLowerCase().includes('anthropic')
                    );
                    if (claudeTerminal) {
                        claudeTerminal.show();
                        void vscode.window.showInformationMessage(
                            `ToneGuard: ${findingCount} fix instructions copied. Press Cmd+V in Claude terminal.`
                        );
                    } else {
                        void vscode.window.showInformationMessage(
                            `ToneGuard: ${findingCount} fix instructions copied. Open Claude Code and paste.`
                        );
                    }
                } else if (choice === 'Fix with Codex') {
                    const prompt = generateFixPrompt(findings);
                    await vscode.env.clipboard.writeText(prompt);
                    // Try to focus Codex terminal if it exists
                    const terminals = vscode.window.terminals;
                    const codexTerminal = terminals.find(t => 
                        t.name.toLowerCase().includes('codex') || 
                        t.name.toLowerCase().includes('openai')
                    );
                    if (codexTerminal) {
                        codexTerminal.show();
                        void vscode.window.showInformationMessage(
                            `ToneGuard: ${findingCount} fix instructions copied. Press Cmd+V in Codex terminal.`
                        );
                    } else {
                        void vscode.window.showInformationMessage(
                            `ToneGuard: ${findingCount} fix instructions copied. Open Codex and paste.`
                        );
                    }
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
            
            // Pre-flight check
            const binaryStatus = getBinaryStatus(context);
            if (!binaryStatus.available) {
                await showBinaryMissingError(binaryStatus);
                return;
            }
            
            const workspaceRoot = workspaceFolders[0].uri.fsPath;
            const reportPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
            const flowsDir = path.join(workspaceRoot, 'flows');
            const args = ['flow', 'audit', '--config', configPath, '--flows', flowsDir, '--out', reportPath, workspaceRoot];

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
            
            // Pre-flight check
            const binaryStatus = getBinaryStatus(context);
            if (!binaryStatus.available) {
                await showBinaryMissingError(binaryStatus);
                return;
            }
            
            const workspaceRoot = workspaceFolders[0].uri.fsPath;
            const reportPath = path.join(workspaceRoot, 'reports', 'flow-proposal.md');
            const flowsDir = path.join(workspaceRoot, 'flows');
            const args = [
                'flow',
                'propose',
                '--config',
                configPath,
                '--flows',
                flowsDir,
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
            
            // Pre-flight check
            const binaryStatus = getBinaryStatus(context);
            if (!binaryStatus.available) {
                await showBinaryMissingError(binaryStatus);
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

    // Command to open Flow Map panel
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.openFlowMap', async () => {
            // Get current file if any
            const activeEditor = vscode.window.activeTextEditor;
            let filePath: string | undefined;
            
            if (activeEditor) {
                const ext = path.extname(activeEditor.document.fileName).toLowerCase();
                if (['.rs', '.ts', '.tsx', '.js', '.jsx', '.py'].includes(ext)) {
                    filePath = activeEditor.document.fileName;
                }
            }

            FlowMapPanel.createOrShow(
                context.extensionUri,
                cliCommand,
                configPath,
                outputChannel,
                filePath
            );
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
