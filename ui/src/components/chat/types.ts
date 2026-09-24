export type MessagePhase = 'thinking' | 'working' | 'streaming' | 'done';

export interface SpecialBlockProps {
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  /** Resend a message whose send failed (the red line's button). */
  onResend?: (failed: import('../../types').ChatMessage) => void;
  inputRef?: React.RefObject<HTMLTextAreaElement | null>;
}
