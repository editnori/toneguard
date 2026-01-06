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
 * Checks if the ToneGuard skill is already installed for the given environment.
 */
function isSkillInstalled(env: 'claude' | 'codex'): boolean {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        return false;
    }

    const workspaceRoot = workspaceFolders[0].uri.fsPath;
    const skillDir = env === 'claude' 
        ? path.join(workspaceRoot, '.claude', 'skills', 'toneguard')
        : path.join(workspaceRoot, '.codex', 'skills', 'toneguard');
    
    return fs.existsSync(path.join(skillDir, 'SKILL.md'));
}

/**
 * Gets the bundled skill template content.
 */
function getSkillTemplate(context: vscode.ExtensionContext): string | undefined {
    const templatePath = path.join(context.extensionPath, 'skills', 'toneguard-skill.md');
    if (fs.existsSync(templatePath)) {
        return fs.readFileSync(templatePath, 'utf8');
    }
    return undefined;
}

/**
 * Installs the ToneGuard skill for the specified AI environment.
 */
async function installSkill(
    context: vscode.ExtensionContext, 
    env: 'claude' | 'codex',
    outputChannel: vscode.OutputChannel
): Promise<boolean> {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        void vscode.window.showErrorMessage(
            'ToneGuard: No workspace folder open. Please open a folder first.'
        );
        return false;
    }

    const workspaceRoot = workspaceFolders[0].uri.fsPath;
    const skillTemplate = getSkillTemplate(context);
    
    if (!skillTemplate) {
        void vscode.window.showErrorMessage(
            'ToneGuard: Could not find skill template. Please reinstall the extension.'
        );
        return false;
    }

    // Determine the target directory
    const baseDir = env === 'claude' ? '.claude' : '.codex';
    const skillDir = path.join(workspaceRoot, baseDir, 'skills', 'toneguard');
    const skillFile = path.join(skillDir, 'SKILL.md');

    try {
        // Create directory structure
        fs.mkdirSync(skillDir, { recursive: true });
        
        // Write the skill file
        fs.writeFileSync(skillFile, skillTemplate, 'utf8');
        
        outputChannel.appendLine(`ToneGuard: Installed skill to ${skillFile}`);
        
        void vscode.window.showInformationMessage(
            `ToneGuard skill installed for ${env === 'claude' ? 'Claude Code' : 'Codex'}! ` +
            `The AI will now follow ToneGuard writing guidelines.`,
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
    if (isSkillInstalled(env)) {
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
        await installSkill(context, env, outputChannel);
    } else if (action === 'Never Ask') {
        await context.globalState.update(SKILL_PROMPT_DISMISSED_KEY, true);
        outputChannel.appendLine('ToneGuard: Skill installation prompt dismissed permanently.');
    }
}

export function activate(context: vscode.ExtensionContext): void {
    const config = vscode.workspace.getConfiguration('dwg');
    const userCommand = config.get<string>('command', 'dwg-lsp');
    const userConfigPath = config.get<string>('configPath', 'layth-style.yml');
    const profile = config.get<string>('profile', '').trim();

    // Resolve actual paths
    const command = getServerCommand(context, userCommand);
    const configPath = getConfigPath(context, userConfigPath);
    
    // Log which server we're using for debugging
    const outputChannel = vscode.window.createOutputChannel('ToneGuard');
    context.subscriptions.push(outputChannel);
    outputChannel.appendLine(`ToneGuard: Using server: ${command}`);
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
    
    // Command to show which server/config is being used
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.showInfo', () => {
            outputChannel.show();
            outputChannel.appendLine('---');
            outputChannel.appendLine(`Platform: ${getPlatformDir()}`);
            outputChannel.appendLine(`Server: ${command}`);
            outputChannel.appendLine(`Config: ${configPath}`);
            outputChannel.appendLine(`Bundled binary exists: ${getBundledServerPath(context) !== undefined}`);
        })
    );

    // Command to install AI skill (auto-detect environment)
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkill', async () => {
            const env = await detectAIEnvironment();
            if (env) {
                await installSkill(context, env, outputChannel);
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
                    await installSkill(context, choice.value, outputChannel);
                }
            }
        })
    );

    // Command to install skill for Claude Code specifically
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkillClaude', async () => {
            await installSkill(context, 'claude', outputChannel);
        })
    );

    // Command to install skill for Codex specifically
    context.subscriptions.push(
        vscode.commands.registerCommand('dwg.installSkillCodex', async () => {
            await installSkill(context, 'codex', outputChannel);
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
