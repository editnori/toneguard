import { JSX } from 'preact';

interface CardProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
    padding?: 'none' | 'sm' | 'md';
}

const paddingClasses = {
    none: '',
    sm: 'p-2',
    md: 'p-3',
};

export function Card({
    children,
    className = '',
    padding = 'md',
    ...props
}: CardProps) {
    return (
        <div
            className={`
                bg-surface border border-border rounded
                ${paddingClasses[padding]}
                ${className}
            `.trim()}
            {...props}
        >
            {children}
        </div>
    );
}

interface CardHeaderProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
}

export function CardHeader({ children, className = '', ...props }: CardHeaderProps) {
    return (
        <div
            className={`flex items-center justify-between gap-2 ${className}`.trim()}
            {...props}
        >
            {children}
        </div>
    );
}

interface CardTitleProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
}

export function CardTitle({ children, className = '', ...props }: CardTitleProps) {
    return (
        <div
            className={`text-[12px] font-semibold text-text ${className}`.trim()}
            {...props}
        >
            {children}
        </div>
    );
}

interface CardContentProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
}

export function CardContent({ children, className = '', ...props }: CardContentProps) {
    return (
        <div className={`mt-2 ${className}`.trim()} {...props}>
            {children}
        </div>
    );
}
