/** @type {import('tailwindcss').Config} */
const contentRoot = __dirname.replace(/\\/g, '/');

module.exports = {
  // Use an absolute-ish path rooted at this config dir so the build works
  // no matter what the current working directory is.
  content: [`${contentRoot}/src/**/*.{ts,tsx,html}`],
  theme: {
    extend: {
      colors: {
        bg: 'var(--tg-bg)',
        surface: 'var(--tg-surface)',
        'surface-2': 'var(--tg-surface-2)',
        text: 'var(--tg-text)',
        muted: 'var(--tg-muted)',
        border: 'var(--tg-border)',
        accent: 'var(--tg-accent)',
        'accent-2': 'var(--tg-accent-2)',
        warn: 'var(--tg-warn)',
        danger: 'var(--tg-danger)',
      },
      fontFamily: {
        sans: ['var(--vscode-font-family)', '-apple-system', 'BlinkMacSystemFont', 'system-ui', 'sans-serif'],
      },
      fontSize: {
        '2xs': ['9px', { lineHeight: '1.2' }],
      },
      spacing: {
        '0.5': '2px',
        '1': '4px',
        '1.5': '6px',
        '2': '8px',
        '2.5': '10px',
        '3': '12px',
      },
      borderRadius: {
        DEFAULT: '3px',
        'sm': '2px',
        'md': '4px',
        'lg': '6px',
      },
      boxShadow: {
        'sm': '0 1px 2px rgba(0, 0, 0, 0.06)',
      },
    },
  },
  plugins: [],
};
