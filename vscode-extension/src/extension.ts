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

type AIEnvironment = 'claude' | 'codex' | null;
type SkillKind = 'writing' | 'logic-flow';

type SidebarNode =
    | { kind: 'section'; id: 'status' | 'actions' | 'reports' | 'findings' }
    | { kind: 'info'; label: string; description?: string }
    | { kind: 'action'; label: string; command: vscode.Command; description?: string }
    | { kind: 'report'; label: string; path: string; command: vscode.Command }
    | { kind: 'category'; category: string; count: number }
    | { kind: 'finding'; finding: any };

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

class ToneGuardSidebarProvider implements vscode.TreeDataProvider<SidebarNode> {
    private readonly onDidChangeEmitter = new vscode.EventEmitter<
        SidebarNode | undefined
    >();
    readonly onDidChangeTreeData = this.onDidChangeEmitter.event;

    constructor(
        private readonly context: vscode.ExtensionContext,
        private readonly serverCommand: string,
        private readonly cliCommand: string,
        private readonly configPath: string
    ) {}

    refresh(): void {
        this.onDidChangeEmitter.fire(undefined);
    }

    getTreeItem(element: SidebarNode): vscode.TreeItem {
        if (element.kind === 'section') {
            const label =
                element.id === 'status'
                    ? 'Status'
                    : element.id === 'actions'
                    ? 'Actions'
                    : element.id === 'reports'
                      ? 'Reports'
                      : 'Findings';
            const item = new vscode.TreeItem(
                label,
                vscode.TreeItemCollapsibleState.Expanded
            );
            item.contextValue = `toneguard.section.${element.id}`;
            return item;
        }

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

        if (element.kind === 'report') {
            const item = new vscode.TreeItem(
                element.label,
                vscode.TreeItemCollapsibleState.None
            );
            item.command = element.command;
            item.description = normalizeReportedPath(element.path);
            item.contextValue = 'toneguard.report';
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

        const finding = element.finding as any;
        const path = typeof finding?.path === 'string' ? finding.path : '<unknown>';
        const line = typeof finding?.line === 'number' ? finding.line : undefined;
        const message =
            typeof finding?.message === 'string' ? finding.message : 'Finding';
        const category =
            typeof finding?.category === 'string' ? finding.category : 'unknown';
        const severity =
            typeof finding?.severity === 'string' ? finding.severity : 'info';

        const loc = line ? `:${line}` : '';
        const label = message.length > 90 ? `${message.slice(0, 87)}...` : message;

        const item = new vscode.TreeItem(
            label,
            vscode.TreeItemCollapsibleState.None
        );
        item.description = `${severity} · ${category} · ${normalizeReportedPath(path)}${loc}`;
        item.tooltip = message;
        item.command = {
            command: 'dwg.openFindingLocation',
            title: 'Open Finding',
            arguments: [path, line],
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

        if (!element) {
            return [
                { kind: 'section', id: 'status' },
                { kind: 'section', id: 'actions' },
                { kind: 'section', id: 'reports' },
                { kind: 'section', id: 'findings' },
            ];
        }

        if (element.kind === 'section') {
            if (element.id === 'status') {
                return this.statusNodes(root);
            }

            if (element.id === 'actions') {
                return [
                    {
                        kind: 'action',
                        label: 'Run Recommended (Audit + Proposal)',
                        command: {
                            command: 'dwg.runRecommended',
                            title: 'Run Recommended',
                        },
                        description: 'Best first step',
                    },
                    {
                        kind: 'action',
                        label: 'Run Flow Audit',
                        command: { command: 'dwg.flowAudit', title: 'Flow Audit' },
                        description: 'Writes reports/flow-audit.json',
                    },
                    {
                        kind: 'action',
                        label: 'Generate Flow Proposal (Markdown)',
                        command: {
                            command: 'dwg.flowPropose',
                            title: 'Flow Propose',
                        },
                        description: 'Writes reports/flow-proposal.md',
                    },
                    {
                        kind: 'action',
                        label: 'New Flow Spec',
                        command: { command: 'dwg.flowNew', title: 'Flow New' },
                        description: 'Scaffold flows/*.md',
                    },
                    {
                        kind: 'action',
                        label: 'Install Logic Flow Guardrails Skill',
                        command: {
                            command: 'dwg.installLogicFlowSkill',
                            title: 'Install Logic Skill',
                        },
                    },
                    {
                        kind: 'action',
                        label: 'Install Writing Style Skill',
                        command: { command: 'dwg.installSkill', title: 'Install Skill' },
                    },
                    {
                        kind: 'action',
                        label: 'Open Docs (README)',
                        command: { command: 'dwg.openDocs', title: 'Open Docs' },
                    },
                    {
                        kind: 'action',
                        label: 'Refresh Sidebar',
                        command: {
                            command: 'dwg.refreshSidebar',
                            title: 'Refresh Sidebar',
                        },
                    },
                ];
            }

            if (element.id === 'reports') {
                return [
                    {
                        kind: 'report',
                        label: 'Open Flow Audit Report',
                        path: 'reports/flow-audit.json',
                        command: {
                            command: 'dwg.openFlowAuditReport',
                            title: 'Open Flow Audit Report',
                        },
                    },
                    {
                        kind: 'report',
                        label: 'Open Flow Proposal',
                        path: 'reports/flow-proposal.md',
                        command: {
                            command: 'dwg.openFlowProposalReport',
                            title: 'Open Flow Proposal',
                        },
                    },
                    {
                        kind: 'action',
                        label: 'Show Server Info',
                        command: { command: 'dwg.showInfo', title: 'Show Info' },
                    },
                ];
            }

            const report = this.readAuditReport(root);
            if (!report) {
                return [
                    {
                        kind: 'action',
                        label: 'No flow audit report found (run Flow Audit)',
                        command: { command: 'dwg.flowAudit', title: 'Flow Audit' },
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
                        kind: 'action',
                        label: 'No findings in last audit',
                        command: { command: 'dwg.flowAudit', title: 'Flow Audit' },
                    },
                ];
            }

            return categories.map(([category, count]) => ({
                kind: 'category',
                category,
                count,
            }));
        }

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

    private statusNodes(workspaceRoot: string): SidebarNode[] {
        const platform = getPlatformDir();
        const bundledCli = getBundledCliPath(this.context);
        const bundledServer = getBundledServerPath(this.context);
        const cliMode = bundledCli ? 'bundled' : 'PATH';
        const serverMode = bundledServer ? 'bundled' : 'PATH';

        const configShown = this.relOrBasename(workspaceRoot, this.configPath);
        const cliShown = this.relOrBasename(workspaceRoot, this.cliCommand);
        const serverShown = this.relOrBasename(workspaceRoot, this.serverCommand);

        const nodes: SidebarNode[] = [
            { kind: 'info', label: `Platform: ${platform}` },
            { kind: 'info', label: `Config: ${configShown}` },
            { kind: 'info', label: `CLI (${cliMode}): ${cliShown}` },
            { kind: 'info', label: `LSP (${serverMode}): ${serverShown}` },
        ];

        const flowsDir = path.join(workspaceRoot, 'flows');
        if (fs.existsSync(flowsDir)) {
            try {
                const files = fs
                    .readdirSync(flowsDir)
                    .filter((name) => name.endsWith('.md') || name.endsWith('.yaml') || name.endsWith('.yml'));
                nodes.push({
                    kind: 'info',
                    label: `Flows: ${files.length} file(s)`,
                    description: 'flows/',
                });
            } catch {
                nodes.push({ kind: 'info', label: 'Flows: (unavailable)' });
            }
        } else {
            nodes.push({ kind: 'info', label: 'Flows: none found', description: 'Create one' });
        }

        const auditPath = path.join(workspaceRoot, 'reports', 'flow-audit.json');
        if (fs.existsSync(auditPath)) {
            const report = this.readAuditReport(workspaceRoot);
            const findings = Array.isArray(report?.audit?.findings)
                ? report.audit.findings
                : [];
            const when = this.fileMtimeLabel(auditPath);
            nodes.push({
                kind: 'info',
                label: `Last audit: ${findings.length} finding(s)`,
                description: when ?? 'reports/flow-audit.json',
            });
        } else {
            nodes.push({ kind: 'info', label: 'Last audit: none', description: 'Run Audit' });
        }

        const proposalPath = path.join(workspaceRoot, 'reports', 'flow-proposal.md');
        if (fs.existsSync(proposalPath)) {
            const when = this.fileMtimeLabel(proposalPath);
            nodes.push({
                kind: 'info',
                label: 'Last proposal: exists',
                description: when ?? 'reports/flow-proposal.md',
            });
        } else {
            nodes.push({
                kind: 'info',
                label: 'Last proposal: none',
                description: 'Generate Proposal',
            });
        }

        if (!bundledCli || !bundledServer) {
            nodes.push({
                kind: 'info',
                label: 'Bundled binaries missing for this platform',
                description: 'Run scripts/install-local.sh',
            });
        }

        return nodes;
    }

    private relOrBasename(workspaceRoot: string, value: string): string {
        try {
            const cleaned = String(value);
            if (!cleaned) {
                return '<unset>';
            }
            const normalized = cleaned.replace(/\\/g, '/');
            if (path.isAbsolute(normalized)) {
                if (normalized.startsWith(workspaceRoot.replace(/\\/g, '/'))) {
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

    private fileMtimeLabel(filePath: string): string | undefined {
        try {
            const stat = fs.statSync(filePath);
            const when = stat.mtime;
            return when.toLocaleString();
        } catch {
            return undefined;
        }
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
 * Detects if the workspace has Claude Code or Codex AI environment markers.
 */
async function detectAIEnvironment(): Promise<AIEnvironment> {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        return null;
    }

    const workspaceRoot = workspaceFolders[0].uri.fsPath;

    // Check for Claude Code markers (.claude directory)
    const claudeDir = path.join(workspaceRoot, '.claude');
    if (fs.existsSync(claudeDir)) {
        return 'claude';
    }

    // Check for Codex markers (.codex directory)
    const codexDir = path.join(workspaceRoot, '.codex');
    if (fs.existsSync(codexDir)) {
        return 'codex';
    }

    return null;
}

/**
 * Gets the global skill directory for the given AI environment.
 * Skills are installed globally so they apply across all projects.
 */
function getGlobalSkillDir(env: 'claude' | 'codex', kind: SkillKind): string {
    const homeDir = os.homedir();
    const baseDir = env === 'claude' ? '.claude' : '.codex';
    return path.join(homeDir, baseDir, 'skills', SKILL_META[kind].dirName);
}

/**
 * Checks if the ToneGuard skill is already installed globally for the given environment.
 */
function isSkillInstalled(env: 'claude' | 'codex', kind: SkillKind): boolean {
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
 * Installs the ToneGuard skill globally for the specified AI environment.
 * Skills are installed in ~/.claude/skills/ or ~/.codex/skills/ so they
 * apply across all projects.
 */
async function installSkill(
    context: vscode.ExtensionContext, 
    env: 'claude' | 'codex',
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

    // Install to global directory (~/.claude/skills/ or ~/.codex/skills/)
    const skillDir = getGlobalSkillDir(env, kind);
    const skillFile = path.join(skillDir, 'SKILL.md');

    try {
        // Create directory structure
        fs.mkdirSync(skillDir, { recursive: true });
        
        // Write the skill file
        fs.writeFileSync(skillFile, skillTemplate, 'utf8');
        
        outputChannel.appendLine(`ToneGuard: Installed ${SKILL_META[kind].label} to ${skillFile}`);
        
        void vscode.window.showInformationMessage(
            `${SKILL_META[kind].label} installed for ${env === 'claude' ? 'Claude Code' : 'Codex'}! ` +
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
 * Prompts the user to install the ToneGuard skill if an AI environment is detected.
 */
async function promptSkillInstallation(
    context: vscode.ExtensionContext,
    env: 'claude' | 'codex',
    outputChannel: vscode.OutputChannel
): Promise<void> {
    // Check if user has dismissed this prompt before
    const dismissed = context.globalState.get<boolean>(SKILL_PROMPT_DISMISSED_KEY, false);
    if (dismissed) {
        return;
    }

    // Check if skill is already installed
    if (isSkillInstalled(env, 'writing')) {
        return;
    }

    const envName = env === 'claude' ? 'Claude Code' : 'Codex';
    const action = await vscode.window.showInformationMessage(
        `ToneGuard detected ${envName}. Install skill to prevent AI slop at the source?`,
        'Install',
        'Not Now',
        'Never Ask'
    );

    if (action === 'Install') {
        await installSkill(context, env, outputChannel, 'writing');
    } else if (action === 'Never Ask') {
        await context.globalState.update(SKILL_PROMPT_DISMISSED_KEY, true);
        outputChannel.appendLine('ToneGuard: Skill installation prompt dismissed permanently.');
    }
}

export function activate(context: vscode.ExtensionContext): void {
    const config = vscode.workspace.getConfiguration('dwg');
    const userCommand = config.get<string>('command', 'dwg-lsp');
    const userCliCommand = config.get<string>('cliCommand', 'dwg');
    const userConfigPath = config.get<string>('configPath', 'layth-style.yml');
    const profile = config.get<string>('profile', '').trim();

    // Resolve actual paths
    const command = getServerCommand(context, userCommand);
    const cliCommand = getCliCommand(context, userCliCommand);
    const configPath = getConfigPath(context, userConfigPath);
    
    // Log which server we're using for debugging
    const outputChannel = vscode.window.createOutputChannel('ToneGuard');
    context.subscriptions.push(outputChannel);
    outputChannel.appendLine(`ToneGuard: Using server: ${command}`);
    outputChannel.appendLine(`ToneGuard: Using CLI: ${cliCommand}`);
    outputChannel.appendLine(`ToneGuard: Using config: ${configPath}`);

    const sidebarProvider = new ToneGuardSidebarProvider(
        context,
        command,
        cliCommand,
        configPath
    );
    context.subscriptions.push(
        vscode.window.registerTreeDataProvider('toneguard.sidebar', sidebarProvider)
    );
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.refreshSidebar', () => {
            sidebarProvider.refresh();
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
    auditWatcher.onDidCreate(() => sidebarProvider.refresh());
    auditWatcher.onDidChange(() => sidebarProvider.refresh());
    auditWatcher.onDidDelete(() => sidebarProvider.refresh());
    context.subscriptions.push(auditWatcher);

    const proposalWatcher = vscode.workspace.createFileSystemWatcher(
        '**/reports/flow-proposal.md'
    );
    proposalWatcher.onDidCreate(() => sidebarProvider.refresh());
    proposalWatcher.onDidChange(() => sidebarProvider.refresh());
    proposalWatcher.onDidDelete(() => sidebarProvider.refresh());
    context.subscriptions.push(proposalWatcher);

    const flowsWatcher = vscode.workspace.createFileSystemWatcher('**/flows/*');
    flowsWatcher.onDidCreate(() => sidebarProvider.refresh());
    flowsWatcher.onDidChange(() => sidebarProvider.refresh());
    flowsWatcher.onDidDelete(() => sidebarProvider.refresh());
    context.subscriptions.push(flowsWatcher);

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

                sidebarProvider.refresh();
                void vscode.window
                    .showInformationMessage(
                        'ToneGuard: Recommended review complete.',
                        'Open Proposal'
                    )
                    .then((selection) => {
                        if (selection === 'Open Proposal') {
                            void vscode.workspace
                                .openTextDocument(proposalPath)
                                .then((doc) => {
                                    void vscode.window.showTextDocument(doc, {
                                        preview: false,
                                    });
                                });
                        }
                    });
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
                        { label: 'Codex', value: 'codex' as const }
                    ],
                    { placeHolder: 'Select AI environment to install skill for' }
                );
                if (choice) {
                    await installSkill(context, choice.value, outputChannel, 'writing');
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
                        { label: 'Codex', value: 'codex' as const }
                    ],
                    { placeHolder: 'Select AI environment to install logic flow skill for' }
                );
                if (choice) {
                    await installSkill(context, choice.value, outputChannel, 'logic-flow');
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

    // Auto-detect AI environment and prompt for skill installation
    void (async () => {
        const env = await detectAIEnvironment();
        if (env) {
            outputChannel.appendLine(`ToneGuard: Detected ${env === 'claude' ? 'Claude Code' : 'Codex'} environment`);
            await promptSkillInstallation(context, env, outputChannel);
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
