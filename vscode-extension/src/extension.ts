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
