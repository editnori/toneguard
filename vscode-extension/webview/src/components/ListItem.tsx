import { JSX } from 'preact';

interface ListItemProps extends JSX.HTMLAttributes<HTMLButtonElement> {
    children: preact.ComponentChildren;
    active?: boolean;
    disabled?: boolean;
}

export function ListItem({
    children,
    active = false,
    disabled = false,
    className = '',
    ...props
}: ListItemProps) {
    return (
        <button
            className={`
                w-full text-left px-2 py-1 text-[11px]
                rounded transition-colors
                ${active ? 'bg-surface-2 text-text' : 'text-muted hover:text-text hover:bg-surface-2'}
                ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                ${className}
            `.trim()}
            disabled={disabled}
            {...props}
        >
            {children}
        </button>
    );
}
