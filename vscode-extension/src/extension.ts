import * as vscode from 'vscode';
import * as path from 'path';
import { spawn } from 'child_process';

interface Location {
    line: number;
    column: number;
}

interface DiagnosticPayload {
    category: string;
    message: string;
    suggestion?: string | null;
    location: Location;
    snippet: string;
}

interface FileResult {
    path: string;
    word_count: number;
    diagnostics: DiagnosticPayload[];
}

interface RepoIssue {
    category: string;
    message: string;
    path?: string | null;
}

interface OutputReport {
    files: FileResult[];
    repo_issues?: RepoIssue[];
}

const collection = vscode.languages.createDiagnosticCollection('dwg');
const pendingTimers = new Map<string, NodeJS.Timeout>();
const trackedUris = new Set<string>();

export function activate(context: vscode.ExtensionContext): void {
    context.subscriptions.push(collection);

    const lintVisible = () => {
        for (const doc of vscode.workspace.textDocuments) {
            if (shouldLint(doc)) {
                scheduleLint(doc);
            }
        }
    };

    context.subscriptions.push(
        vscode.workspace.onDidOpenTextDocument((doc) => {
            if (shouldLint(doc)) {
                scheduleLint(doc);
            }
        }),
        vscode.workspace.onDidChangeTextDocument((event) => {
            if (shouldLint(event.document)) {
                scheduleLint(event.document);
            }
        }),
        vscode.workspace.onDidSaveTextDocument((doc) => {
            if (shouldLint(doc)) {
                scheduleLint(doc, true);
            }
        }),
        vscode.workspace.onDidChangeConfiguration((event) => {
            if (event.affectsConfiguration('dwg')) {
                lintVisible();
            }
        }),
        vscode.commands.registerCommand('dwg.lintWorkspace', () => lintWorkspace())
    );

    lintVisible();
}

export function deactivate(): void {
    collection.dispose();
    pendingTimers.forEach((timer) => clearTimeout(timer));
    pendingTimers.clear();
    trackedUris.clear();
}

function shouldLint(document: vscode.TextDocument): boolean {
    if (document.isUntitled) {
        return false;
    }
    return document.languageId === 'markdown' || document.languageId === 'plaintext';
}

function scheduleLint(document: vscode.TextDocument, immediate = false): void {
    const key = document.uri.toString();
    const configuration = vscode.workspace.getConfiguration();
    const debounce = configuration.get<number>('dwg.debounceMs', 400);
    if (pendingTimers.has(key)) {
        clearTimeout(pendingTimers.get(key));
    }
    const timer = setTimeout(() => {
        pendingTimers.delete(key);
        runLint(document).catch((error: unknown) => {
            console.error('dwg lint failed', error);
        });
    }, immediate ? 0 : debounce);
    pendingTimers.set(key, timer);
}

async function runLint(document: vscode.TextDocument): Promise<void> {
    const configuration = vscode.workspace.getConfiguration();
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
    const cwd = workspaceFolder?.uri.fsPath ?? path.dirname(document.uri.fsPath);
    const invocation = buildCliInvocation(configuration, document.uri.fsPath, false);
    const { stdout, stderr, exitCode } = await execCommand(invocation.command, invocation.args, cwd);

    if (!stdout.trim()) {
        if (stderr.trim()) {
            void vscode.window.showErrorMessage(`DWG: ${stderr.trim()}`);
        }
        updateDiagnostics(document.uri, []);
        return;
    }

    let payload: OutputReport;
    try {
        payload = JSON.parse(stdout) as OutputReport;
    } catch (error) {
        void vscode.window.showErrorMessage('DWG failed to parse JSON output. See console.');
        console.error('DWG JSON parse error', error, stdout);
        return;
    }

    const matching = payload.files.find((file) => {
        return path.normalize(file.path) === path.normalize(document.uri.fsPath);
    });

    if (!matching) {
        updateDiagnostics(document.uri, []);
        return;
    }

    const diagnostics = matching.diagnostics.map(toDiagnostic);
    updateDiagnostics(document.uri, diagnostics);

    if (exitCode !== 0) {
        const warning = `DWG density threshold exceeded (${matching.diagnostics.length} issues).`;
        void vscode.window.showWarningMessage(warning);
    }
}

async function lintWorkspace(): Promise<void> {
    const configuration = vscode.workspace.getConfiguration();
    const folders = vscode.workspace.workspaceFolders;
    if (!folders || folders.length === 0) {
        void vscode.window.showInformationMessage('DWG: No workspace folder to lint.');
        return;
    }
    const folder = folders[0];
    const cwd = folder.uri.fsPath;
    const invocation = buildCliInvocation(configuration, cwd, true);
    const { stdout, stderr } = await execCommand(invocation.command, invocation.args, cwd);
    if (!stdout.trim()) {
        if (stderr.trim()) {
            void vscode.window.showErrorMessage(`DWG: ${stderr.trim()}`);
        }
        return;
    }
    let payload: OutputReport;
    try {
        payload = JSON.parse(stdout) as OutputReport;
    } catch (error) {
        void vscode.window.showErrorMessage('DWG workspace lint failed to parse JSON output. See console.');
        console.error('DWG JSON parse error', error, stdout);
        return;
    }

    applyWorkspaceDiagnostics(payload, cwd);

    if (payload.repo_issues && payload.repo_issues.length) {
        const preview = payload.repo_issues
            .slice(0, 5)
            .map((issue) => (issue.path ? `${issue.path}: ${issue.message}` : issue.message))
            .join('\n');
        void vscode.window.showInformationMessage(
            `DWG repo issues (${payload.repo_issues.length}). See terminal for full list.`
        );
    }
}

function buildCliInvocation(
    configuration: vscode.WorkspaceConfiguration,
    targetPath: string,
    forceStrict: boolean
): { command: string; args: string[] } {
    const command = configuration.get<string>('dwg.command', 'dwg-cli');
    const configPath = configuration.get<string>('dwg.configPath', 'layth-style.yml');
    const args = ['--json', '--quiet', '--config', configPath];

    const forcedProfile = configuration.get<string>('dwg.profile', '').trim();
    if (forcedProfile) {
        args.push('--profile', forcedProfile);
    }
    const strict = forceStrict || configuration.get<boolean>('dwg.strict', false);
    if (strict) {
        args.push('--strict');
    }
    const noRepo = configuration.get<boolean>('dwg.noRepoChecks', false);
    if (noRepo) {
        args.push('--no-repo-checks');
    }
    const onlyCats = configuration.get<string[]>('dwg.onlyCategories', []) ?? [];
    const enableCats = configuration.get<string[]>('dwg.enableCategories', []) ?? [];
    const disableCats = configuration.get<string[]>('dwg.disableCategories', []) ?? [];
    if (onlyCats.length) {
        args.push('--only', onlyCats.join(','));
    }
    if (enableCats.length) {
        args.push('--enable', enableCats.join(','));
    }
    if (disableCats.length) {
        args.push('--disable', disableCats.join(','));
    }
    const sets = configuration.get<string[]>('dwg.set', []) ?? [];
    for (const s of sets) {
        if (s && s.includes('=')) {
            args.push('--set', s);
        }
    }
    const onlyRepo = configuration.get<string[]>('dwg.onlyRepoCategories', []) ?? [];
    const enableRepo = configuration.get<string[]>('dwg.enableRepoCategories', []) ?? [];
    const disableRepo = configuration.get<string[]>('dwg.disableRepoCategories', []) ?? [];
    if (onlyRepo.length) {
        args.push('--only-repo', onlyRepo.join(','));
    }
    if (enableRepo.length) {
        args.push('--enable-repo', enableRepo.join(','));
    }
    if (disableRepo.length) {
        args.push('--disable-repo', disableRepo.join(','));
    }

    args.push(targetPath);
    return { command, args };
}

function toDiagnostic(diag: DiagnosticPayload): vscode.Diagnostic {
    const startLine = Math.max(0, diag.location.line - 1);
    const startCol = Math.max(0, diag.location.column - 1);
    const snippetLength = Math.max(diag.snippet ? diag.snippet.length : 0, 1);
    const range = new vscode.Range(startLine, startCol, startLine, startCol + snippetLength);
    let message = `[${diag.category}] ${diag.message}`;
    if (diag.suggestion) {
        message += ` → ${diag.suggestion}`;
    }
    const diagnostic = new vscode.Diagnostic(range, message, vscode.DiagnosticSeverity.Warning);
    diagnostic.source = 'dwg';
    return diagnostic;
}

function updateDiagnostics(uri: vscode.Uri, diagnostics: vscode.Diagnostic[]): void {
    collection.set(uri, diagnostics);
    const key = uri.toString();
    if (diagnostics.length === 0) {
        trackedUris.delete(key);
    } else {
        trackedUris.add(key);
    }
}

function applyWorkspaceDiagnostics(payload: OutputReport, workspaceRoot: string): void {
    const updated = new Set<string>();
    for (const file of payload.files) {
        const absolutePath = path.isAbsolute(file.path)
            ? file.path
            : path.join(workspaceRoot, file.path);
        const uri = vscode.Uri.file(path.normalize(absolutePath));
        const diagnostics = file.diagnostics.map(toDiagnostic);
        updateDiagnostics(uri, diagnostics);
        updated.add(uri.toString());
    }

    for (const key of Array.from(trackedUris)) {
        if (!updated.has(key)) {
            collection.set(vscode.Uri.parse(key), []);
            trackedUris.delete(key);
        }
    }
}

function execCommand(
    command: string,
    args: string[],
    cwd: string
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, { cwd, shell: false });
        const stdoutChunks: Buffer[] = [];
        const stderrChunks: Buffer[] = [];

        child.stdout.on('data', (chunk) => stdoutChunks.push(chunk));
        child.stderr.on('data', (chunk) => stderrChunks.push(chunk));
        child.on('error', (error) => reject(error));
        child.on('close', (code) => {
            resolve({
                stdout: Buffer.concat(stdoutChunks).toString('utf8'),
                stderr: Buffer.concat(stderrChunks).toString('utf8'),
                exitCode: code ?? 0,
            });
        });
    });
}
