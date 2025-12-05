import React, { useEffect, useRef } from 'react';

interface ContextMenuProps {
    x: number;
    y: number;
    onClose: () => void;
    children: React.ReactNode;
}

export const ContextMenu: React.FC<ContextMenuProps> = ({ x, y, onClose, children }) => {
    const menuRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
                onClose();
            }
        };

        const handleEscape = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                onClose();
            }
        };

        const handleRightClick = (e: MouseEvent) => {
            // Close if right-clicking elsewhere, let parent handle reopening
            if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
                onClose();
            }
        };

        document.addEventListener('mousedown', handleClickOutside);
        document.addEventListener('keydown', handleEscape);
        document.addEventListener('contextmenu', handleRightClick);

        return () => {
            document.removeEventListener('mousedown', handleClickOutside);
            document.removeEventListener('keydown', handleEscape);
            document.removeEventListener('contextmenu', handleRightClick);
        };
    }, [onClose]);

    // Adjust position if it spills off-screen (basic version)
    // We can enhance this to measure window size

    return (
        <div
            ref={menuRef}
            className="context-menu"
            style={{
                position: 'fixed',
                top: y,
                left: x,
                zIndex: 1000, // Above everything
                backgroundColor: 'var(--bg-content)',
                border: '1px solid var(--border-color)',
                borderRadius: '6px',
                boxShadow: '0 4px 12px rgba(0,0,0,0.2)',
                padding: '4px 0',
                minWidth: '160px',
                display: 'flex',
                flexDirection: 'column'
            }}
            onClick={(e) => e.stopPropagation()}
            onContextMenu={(e) => e.preventDefault()}
        >
            {children}
        </div>
    );
};

interface ContextMenuItemProps {
    onClick: () => void;
    label: string;
    icon?: React.ReactNode;
    color?: string;
    danger?: boolean;
}

export const ContextMenuItem: React.FC<ContextMenuItemProps> = ({ onClick, label, icon, color, danger }) => {
    return (
        <button
            className="context-menu-item"
            onClick={(e) => {
                e.stopPropagation(); // Prevent Sidebar click
                onClick();
            }}
            style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '8px 12px',
                width: '100%',
                border: 'none',
                background: 'transparent',
                color: danger ? 'var(--error)' : (color || 'var(--text-primary)'),
                cursor: 'pointer',
                textAlign: 'left',
                fontSize: '0.85rem'
            }}
            onMouseOver={(e) => e.currentTarget.style.background = 'var(--bg-secondary)'}
            onMouseOut={(e) => e.currentTarget.style.background = 'transparent'}
        >
            {icon && <span style={{ width: '16px', height: '16px', display: 'flex', alignItems: 'center' }}>{icon}</span>}
            <span>{label}</span>
        </button>
    );
};
