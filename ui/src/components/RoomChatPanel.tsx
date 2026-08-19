/**
 * Room chat panel — floating in the left sidebar.
 *
 * Two states:
 * - Collapsed: small card showing room name + latest message + unread badge
 * - Expanded: message list + input, takes lower portion of sidebar
 *
 * Only visible when user is in a room (owner with active room, or consumer).
 */
import React, { useRef, useEffect, useState, useCallback } from 'react';
import { MessageCircle, Send, ChevronDown, ChevronRight, Settings } from 'lucide-react';
import { useRoomChatStore } from '../stores/roomChatStore';
import { useUserStore } from '../stores/userStore';
import { useOpenSettings } from '../hooks/useOpenSettings';
import { getTransport } from '../lib/transport';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Format epoch-ms to short time string. */
function shortTime(ms: number): string {
  const d = new Date(ms);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** Generate initials from a name. */
function initials(name: string): string {
  return [...name].slice(0, 2).join('').toUpperCase();
}

/** Pick a consistent colour for a sender. */
function avatarColor(senderId: string): string {
  const colors = [
    'bg-blue-500', 'bg-emerald-500', 'bg-amber-500', 'bg-purple-500',
    'bg-pink-500', 'bg-cyan-500', 'bg-rose-500', 'bg-indigo-500',
  ];
  let hash = 0;
  for (let i = 0; i < senderId.length; i++) hash = (hash * 31 + senderId.charCodeAt(i)) | 0;
  return colors[Math.abs(hash) % colors.length];
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const RoomChatPanel: React.FC = () => {
  const messages = useRoomChatStore((s) => s.messages);
  const isOpen = useRoomChatStore((s) => s.isOpen);
  const unreadCount = useRoomChatStore((s) => s.unreadCount);
  const setOpen = useRoomChatStore((s) => s.setOpen);
  const ownerRoomName = useUserStore((s) => s.userRoomName);
  const proxyRoomName = useUserStore((s) => s.proxyRoomName);
  const roomEnabled = useUserStore((s) => s.roomEnabled);
  const roomName = ownerRoomName || proxyRoomName;
  const openSettings = useOpenSettings();

  const [input, setInput] = useState('');
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    if (isOpen && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [messages.length, isOpen]);

  // Focus input when expanded
  useEffect(() => {
    if (isOpen) inputRef.current?.focus();
  }, [isOpen]);

  // Click outside → collapse
  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [isOpen, setOpen]);

  // Esc → collapse
  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [isOpen, setOpen]);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text) return;
    const transport = getTransport();
    const { userName: storedName } = useUserStore.getState();
    const userName = storedName || 'Me';
    transport.sendRoomChat?.(text, userName);
    setInput('');
    inputRef.current?.focus();
  }, [input]);

  // Show panel if owner has room enabled or if consumer has a room name
  if (!roomName && !roomEnabled) return null;

  const lastMsg = messages.length > 0 ? messages[messages.length - 1] : null;

  // ── Collapsed card ──
  if (!isOpen) {
    return (
      <div
        className="border-t border-slate-200 dark:border-white/5 px-3 py-2 cursor-pointer
                   hover:bg-slate-50 dark:hover:bg-white/[0.03] select-none flex items-center gap-2"
        onClick={() => setOpen(true)}
      >
        <MessageCircle size={14} className="text-slate-400 shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1">
            <ChevronRight size={10} className="text-slate-400" />
            <span className="text-[10px] font-bold uppercase tracking-widest text-slate-400">Room Chat</span>
            <div className="ml-auto flex items-center gap-1">
              <button onClick={(e) => { e.stopPropagation(); openSettings('room'); }}
                className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-blue-500 transition-colors" title="Room settings">
                <Settings size={11} />
              </button>
              {unreadCount > 0 && (
                <span className="text-[9px] font-bold bg-blue-500 text-white rounded-full px-1.5 min-w-[16px] text-center">
                  {unreadCount > 99 ? '99+' : unreadCount}
                </span>
              )}
            </div>
          </div>
          {lastMsg && (
            <p className="text-[11px] text-slate-500 dark:text-slate-500 truncate mt-0.5">
              <span className="font-medium text-slate-600 dark:text-slate-400">{lastMsg.senderName}:</span> {lastMsg.text}
            </p>
          )}
        </div>
      </div>
    );
  }

  // ── Expanded panel ──
  return (
    <div ref={panelRef} className="border-t border-slate-200 dark:border-white/5 flex flex-col" style={{ maxHeight: '40%' }}>
      {/* Header */}
      <div
        className="flex items-center gap-1 px-3 py-1.5 bg-slate-50/50 dark:bg-white/[0.02] cursor-pointer select-none"
        onClick={() => setOpen(false)}
      >
        <ChevronDown size={10} className="text-slate-400" />
        <span className="text-[10px] font-bold uppercase tracking-widest text-slate-400">Room Chat</span>
        <div className="ml-auto flex items-center gap-1.5">
          <button onClick={(e) => { e.stopPropagation(); openSettings('room'); }}
            className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-blue-500 transition-colors" title="Room settings">
            <Settings size={11} />
          </button>
          <span className="text-[10px] text-slate-400">{messages.length}</span>
        </div>
      </div>

      {/* Messages */}
      <div ref={listRef} className="flex-1 overflow-y-auto min-h-0 px-2 py-1 space-y-1.5">
        {messages.length === 0 && (
          <p className="text-[11px] text-slate-400 text-center py-4">No messages yet</p>
        )}
        {messages.map((msg) => (
          <div key={msg.id} className={`flex items-start gap-1.5 ${msg.isMine ? 'flex-row-reverse' : ''}`}>
            {msg.avatarUrl ? (
              <img src={msg.avatarUrl} alt={msg.senderName} className="shrink-0 w-5 h-5 rounded-full object-cover" />
            ) : (
              <div className={`shrink-0 w-5 h-5 rounded-full flex items-center justify-center text-[8px] font-bold text-white ${avatarColor(msg.senderId)}`}>
                {initials(msg.senderName)}
              </div>
            )}
            <div className={`max-w-[80%] ${msg.isMine ? 'items-end' : 'items-start'}`}>
              {!msg.isMine && (
                <span className="text-[9px] text-slate-400 ml-0.5">{msg.senderName}</span>
              )}
              <div className={`text-[12px] px-2 py-1 rounded-lg ${
                msg.isMine
                  ? 'bg-blue-500 text-white rounded-tr-sm'
                  : 'bg-slate-100 dark:bg-white/[0.06] text-slate-800 dark:text-slate-200 rounded-tl-sm'
              }`}>
                {msg.text}
              </div>
              <span className="text-[9px] text-slate-400 ml-0.5">{shortTime(msg.timestamp)}</span>
            </div>
          </div>
        ))}
      </div>

      {/* Input */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-t border-slate-100 dark:border-white/[0.04]">
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.nativeEvent.isComposing || e.keyCode === 229) return; if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
          placeholder="Type a message..."
          className="flex-1 text-[12px] bg-transparent border-none outline-none text-slate-800 dark:text-slate-200 placeholder-slate-400"
        />
        <button
          onClick={handleSend}
          disabled={!input.trim()}
          className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/10 text-slate-400 hover:text-blue-500 disabled:opacity-30 transition-colors"
        >
          <Send size={12} />
        </button>
      </div>
    </div>
  );
};
