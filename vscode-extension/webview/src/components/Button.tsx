import { JSX } from 'preact';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md';

interface ButtonProps extends Omit<JSX.HTMLAttributes<HTMLButtonElement>, 'size'> {
    variant?: ButtonVariant;
    size?: ButtonSize;
    children: preact.ComponentChildren;
    disabled?: boolean;
}

const variantClasses: Record<ButtonVariant, string> = {
    primary: 'bg-accent text-white hover:bg-accent/90',
    secondary: 'bg-surface-2 text-text border border-border hover:border-accent',
    ghost: 'bg-transparent text-muted hover:text-text hover:bg-surface-2',
    danger: 'bg-transparent text-danger hover:bg-danger/10',
};

const sizeClasses: Record<ButtonSize, string> = {
    sm: 'px-2 py-0.5 text-[11px]',
    md: 'px-2.5 py-1 text-[12px]',
};

export function Button({
    variant = 'secondary',
    size = 'md',
    children,
    className = '',
    disabled,
    ...props
}: ButtonProps) {
    return (
        <button
            className={`
                inline-flex items-center justify-center gap-1
                rounded font-medium transition-colors duration-100
                focus:outline-none focus:ring-1 focus:ring-accent/50
                disabled:opacity-50 disabled:cursor-not-allowed
                ${variantClasses[variant]}
                ${sizeClasses[size]}
                ${className}
            `.trim()}
            disabled={disabled}
            {...props}
        >
            {children}
        </button>
    );
}
