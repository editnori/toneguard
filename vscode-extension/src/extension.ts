import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
    const config = vscode.workspace.getConfiguration('dwg');
    const command = config.get<string>('command', 'dwg-lsp');
    const configPath = config.get<string>('configPath', 'layth-style.yml');
    const profile = config.get<string>('profile', '').trim();

    const serverOptions: ServerOptions = {
        command,
        args: [],
    };

    const fileEvents: vscode.FileSystemWatcher[] = [];
    if (configPath && !path.isAbsolute(configPath)) {
        fileEvents.push(vscode.workspace.createFileSystemWatcher(`**/${configPath}`));
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
    void client.start();

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
}

export async function deactivate(): Promise<void> {
    if (!client) {
        return;
    }
    await client.stop();
    client = undefined;
}
