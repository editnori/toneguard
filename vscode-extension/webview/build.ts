#!/usr/bin/env bun
/**
 * Build script for ToneGuard webviews
 * Creates completely standalone bundles with no external imports
 * to work reliably in VS Code webviews with strict CSP
 */
import { build } from 'esbuild';
import * as path from 'path';
import * as fs from 'fs';
import { $ } from 'bun';

const __dirname = import.meta.dir;
const outDir = path.resolve(__dirname, '..', 'media');

// Ensure output directory exists
fs.mkdirSync(outDir, { recursive: true });

// Clean old files
for (const file of fs.readdirSync(outDir)) {
    if (file.endsWith('.js') || file.endsWith('.css')) {
        fs.unlinkSync(path.join(outDir, file));
    }
}
// Clean chunks/assets folders
for (const subdir of ['chunks', 'assets']) {
    const subPath = path.join(outDir, subdir);
    if (fs.existsSync(subPath)) {
        fs.rmSync(subPath, { recursive: true });
    }
}

const commonOptions = {
    bundle: true,
    minify: true,
    target: 'es2020',
    jsx: 'automatic' as const,
    jsxImportSource: 'preact',
    define: {
        'process.env.NODE_ENV': '"production"',
    },
    loader: {
        '.tsx': 'tsx' as const,
        '.ts': 'ts' as const,
        // Never emit runtime CSS imports in the webview bundle.
        // CSS is built separately via Tailwind and loaded via <link>.
        '.css': 'empty' as const,
    },
};

async function buildWebviews() {
    console.log('Building dashboard...');
    await build({
        ...commonOptions,
        entryPoints: [path.resolve(__dirname, 'src/dashboard/main.tsx')],
        outfile: path.join(outDir, 'dashboard.js'),
        format: 'iife',
    });

    console.log('Building flowmap...');
    await build({
        ...commonOptions,
        entryPoints: [path.resolve(__dirname, 'src/flowmap/main.tsx')],
        outfile: path.join(outDir, 'flowmap.js'),
        format: 'iife',
    });

    // Build CSS with Tailwind CLI
    console.log('Building styles with Tailwind...');
    const cssInput = path.resolve(__dirname, 'src/styles.css');
    const cssOutput = path.join(outDir, 'style.css');
    
    await $`bunx tailwindcss -c ${path.resolve(__dirname, 'tailwind.config.cjs')} -i ${cssInput} -o ${cssOutput} --minify`.quiet();

    // Get file sizes
    const files = ['dashboard.js', 'flowmap.js', 'style.css'];
    console.log('\nOutput:');
    for (const file of files) {
        const filePath = path.join(outDir, file);
        if (fs.existsSync(filePath)) {
            const stat = fs.statSync(filePath);
            const sizeKb = (stat.size / 1024).toFixed(2);
            console.log(`  ${file}: ${sizeKb} KB`);
        }
    }

    console.log('\n✓ Build complete');
}

buildWebviews().catch((err) => {
    console.error('Build failed:', err);
    process.exit(1);
});
