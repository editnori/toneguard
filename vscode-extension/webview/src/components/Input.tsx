import { JSX } from 'preact';

interface InputProps extends Omit<JSX.HTMLAttributes<HTMLInputElement>, 'size'> {
    size?: 'sm' | 'md';
    type?: string;
    placeholder?: string;
    value?: string | number;
    disabled?: boolean;
}

const sizeClasses = {
    sm: 'px-1.5 py-0.5 text-[11px]',
    md: 'px-2 py-1 text-[12px]',
};

export function Input({
    size = 'md',
    className = '',
    ...props
}: InputProps) {
    return (
        <input
            className={`
                bg-surface-2 border border-border rounded
                text-text placeholder:text-muted
                focus:outline-none focus:border-accent
                ${sizeClasses[size]}
                ${className}
            `.trim()}
            {...props}
        />
    );
}

interface SelectProps extends JSX.HTMLAttributes<HTMLSelectElement> {
    children: preact.ComponentChildren;
    value?: string;
    disabled?: boolean;
}

export function Select({ className = '', children, ...props }: SelectProps) {
    return (
        <select
            className={`
                bg-surface-2 border border-border rounded
                px-2 py-1 text-[12px] text-text
                focus:outline-none focus:border-accent
                ${className}
            `.trim()}
            {...props}
        >
            {children}
        </select>
    );
}
