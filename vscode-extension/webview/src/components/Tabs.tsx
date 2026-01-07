import { Signal, useSignal } from '@preact/signals';
import { JSX } from 'preact';

interface TabsProps {
    tabs: { id: string; label: string }[];
    activeTab: Signal<string>;
    onChange?: (id: string) => void;
}

export function Tabs({ tabs, activeTab, onChange }: TabsProps) {
    return (
        <div className="flex items-center gap-0 border-b border-border">
            {tabs.map((tab) => {
                const isActive = activeTab.value === tab.id;
                return (
                    <button
                        key={tab.id}
                        onClick={() => {
                            activeTab.value = tab.id;
                            onChange?.(tab.id);
                        }}
                        className={`
                            px-3 py-1.5 text-[12px] font-medium transition-colors
                            border-b-2 -mb-px
                            ${isActive
                                ? 'text-text border-accent'
                                : 'text-muted border-transparent hover:text-text hover:border-border'
                            }
                        `.trim()}
                    >
                        {tab.label}
                    </button>
                );
            })}
        </div>
    );
}

interface TabPanelProps extends JSX.HTMLAttributes<HTMLDivElement> {
    id: string;
    activeTab: Signal<string>;
    children: preact.ComponentChildren;
}

export function TabPanel({ id, activeTab, children, className = '', ...props }: TabPanelProps) {
    if (activeTab.value !== id) return null;
    return (
        <div className={`mt-3 ${className}`.trim()} {...props}>
            {children}
        </div>
    );
}
