import { JSX } from 'preact';

interface RowProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
    justify?: 'start' | 'between' | 'end' | 'center';
    align?: 'start' | 'center' | 'end';
    gap?: 'none' | 'xs' | 'sm' | 'md';
}

const justifyClasses = {
    start: 'justify-start',
    between: 'justify-between',
    end: 'justify-end',
    center: 'justify-center',
};

const alignClasses = {
    start: 'items-start',
    center: 'items-center',
    end: 'items-end',
};

const gapClasses = {
    none: 'gap-0',
    xs: 'gap-1',
    sm: 'gap-2',
    md: 'gap-3',
};

export function Row({
    children,
    justify = 'start',
    align = 'center',
    gap = 'sm',
    className = '',
    ...props
}: RowProps) {
    return (
        <div
            className={`
                flex flex-wrap
                ${justifyClasses[justify]}
                ${alignClasses[align]}
                ${gapClasses[gap]}
                ${className}
            `.trim()}
            {...props}
        >
            {children}
        </div>
    );
}

interface StackProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children: preact.ComponentChildren;
    gap?: 'none' | 'xs' | 'sm' | 'md';
}

export function Stack({
    children,
    gap = 'sm',
    className = '',
    ...props
}: StackProps) {
    return (
        <div
            className={`flex flex-col ${gapClasses[gap]} ${className}`.trim()}
            {...props}
        >
            {children}
        </div>
    );
}
