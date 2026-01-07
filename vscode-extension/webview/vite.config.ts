import { defineConfig } from 'vite';
import * as path from 'path';

// Build config for VS Code webviews
// Uses ES modules but avoids code splitting for simpler webview loading
export default defineConfig({
    root: __dirname,
    esbuild: {
        jsx: 'automatic',
        jsxImportSource: 'preact',
    },
    build: {
        outDir: path.resolve(__dirname, '..', 'media'),
        emptyOutDir: true,
        cssCodeSplit: false,
        sourcemap: false,
        minify: 'esbuild',
        target: 'es2020',
        rollupOptions: {
            input: {
                dashboard: path.resolve(__dirname, 'src/dashboard/main.tsx'),
                flowmap: path.resolve(__dirname, 'src/flowmap/main.tsx'),
            },
            output: {
                format: 'es',
                entryFileNames: '[name].js',
                // Disable code splitting - each entry gets all its deps inlined
                manualChunks: () => undefined,
                assetFileNames: '[name][extname]',
            },
        },
    },
    define: {
        'process.env.NODE_ENV': JSON.stringify('production'),
    },
});
