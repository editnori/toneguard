import { JSX } from 'preact';

type BadgeVariant = 'default' | 'success' | 'warning' | 'danger' | 'info';

interface BadgeProps extends JSX.HTMLAttributes<HTMLSpanElement> {
    variant?: BadgeVariant;
    children: preact.ComponentChildren;
}

const variantClasses: Record<BadgeVariant, string> = {
    default: 'text-muted border-border',
    success: 'text-accent-2 border-accent-2',
    warning: 'text-warn border-warn',
    danger: 'text-danger border-danger',
    info: 'text-accent border-accent',
};

export function Badge({
    variant = 'default',
    children,
    className = '',
    ...props
}: BadgeProps) {
    return (
        <span
            className={`
                inline-flex items-center
                px-1.5 py-0.5 text-[10px] font-medium
                border rounded
                ${variantClasses[variant]}
                ${className}
            `.trim()}
            {...props}
        >
            {children}
        </span>
    );
}
