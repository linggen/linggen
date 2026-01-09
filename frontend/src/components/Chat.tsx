import { useState, useRef, useEffect } from 'react'
import { chatStream } from '../api'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

interface ChatProps {
  llmEnabled: boolean
}

// Typing effect configuration
const TYPING_INTERVAL_MS = 25
const CHARS_PER_TICK = 2

export function Chat({ llmEnabled }: ChatProps) {
  const [messages, setMessages] = useState<Message[]>([
    { role: 'assistant', content: 'Hello! I can help you understand your codebase. Ask me anything!' }
  ])
  const [input, setInput] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)

  // Queue of characters to display for the current assistant message
  const charQueueRef = useRef<string[]>([])

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  // Auto-scroll as messages update
  useEffect(() => {
    scrollToBottom()
  }, [messages, isLoading])

  // Global typing loop: every TYPING_INTERVAL_MS, flush a few characters from the queue
  useEffect(() => {
    const interval = window.setInterval(() => {
      if (charQueueRef.current.length === 0) return

      const chunk = charQueueRef.current.splice(0, CHARS_PER_TICK).join('')

      setMessages(prev => {
        const lastIdx = prev.length - 1
        if (lastIdx >= 0 && prev[lastIdx].role === 'assistant') {
          return [
            ...prev.slice(0, lastIdx),
            { ...prev[lastIdx], content: prev[lastIdx].content + chunk },
          ]
        }
        return prev
      })
    }, TYPING_INTERVAL_MS)

    return () => window.clearInterval(interval)
  }, [])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!input.trim() || isLoading) return

    const userMessage = input.trim()
    setInput('')

    // Add user message
    setMessages(prev => [...prev, { role: 'user', content: userMessage }])
    setIsLoading(true)

    // Reset any leftover characters for previous responses
    charQueueRef.current = []

    // Add initial empty assistant message that we will stream into
    setMessages(prev => [...prev, { role: 'assistant', content: '' }])

    try {
      await chatStream(userMessage, (token) => {
        // Push characters into the queue; the typing loop will flush them to the UI
        if (token) {
          charQueueRef.current.push(...token.split(''))
        }
      })
    } catch (error) {
      setMessages(prev => {
        const newMessages = [...prev]
        const lastMsg = newMessages[newMessages.length - 1]
        if (lastMsg.role === 'assistant') {
          if (!lastMsg.content) {
            lastMsg.content = `Error: ${error}`
          } else {
            lastMsg.content += `\n[Error: ${error}]`
          }
        }
        return newMessages
      })
    } finally {
      setIsLoading(false)
    }
  }

  // If LLM is disabled, show a message instead of the chat interface
  if (!llmEnabled) {
    return (
      <div className="flex flex-col h-[600px] bg-black/20 rounded-xl border border-[var(--border-color)] overflow-hidden">
        <div className="p-4 bg-white/2 border-b border-white/5">
          <h3 className="m-0 text-base text-[var(--text-active)]">💬 Quick Chat</h3>
          <p className="m-0 mt-1 text-[10px] text-[var(--text-secondary)] uppercase tracking-wider">Powered by Qwen3-4B</p>
        </div>
        <div className="flex-1 flex items-center justify-center p-8 text-center">
          <div className="max-w-[400px]">
            <div className="text-5xl mb-4 opacity-20 grayscale">🔒</div>
            <h4 className="text-[var(--text-active)] font-semibold">Chat Disabled</h4>
            <p className="mt-2 text-sm text-[var(--text-secondary)] leading-relaxed">
              The local LLM is currently disabled. Enable it in Settings to use the chat feature.
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-[600px] bg-black/20 rounded-xl border border-[var(--border-color)] overflow-hidden">
      <div className="p-4 bg-white/2 border-b border-white/5">
        <h3 className="m-0 text-base text-[var(--text-active)] font-semibold">💬 Quick Chat</h3>
        <p className="m-0 mt-1 text-[10px] text-[var(--text-secondary)] uppercase tracking-wider">Powered by Qwen3-4B · No Context</p>
      </div>

      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-4">
        {messages.map((msg, idx) => (
          <div key={idx} className={`flex flex-col max-w-[85%] ${msg.role === 'user' ? 'self-end items-end' : 'self-start items-start'}`}>
            <div className={`p-3 px-4 rounded-2xl text-[0.9rem] leading-relaxed break-words ${
              msg.role === 'user' 
                ? 'bg-[var(--accent)] text-white rounded-br-none shadow-sm' 
                : 'bg-white/10 text-[var(--text-primary)] rounded-bl-none'
            }`}>
              {msg.content || (isLoading && idx === messages.length - 1 ? '...' : '')}
            </div>
          </div>
        ))}
        <div ref={messagesEndRef} />
      </div>

      <form onSubmit={handleSubmit} className="p-4 bg-white/2 border-t border-white/5 flex gap-2">
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Ask a question..."
          disabled={isLoading}
          className="flex-1 p-2 px-3 bg-black/30 border border-white/10 rounded-md text-[var(--text-primary)] text-[0.9rem] outline-none focus:border-[var(--accent)] transition-all"
        />
        <button type="submit" disabled={isLoading || !input.trim()} className="btn-primary">
          Send
        </button>
      </form>
    </div>
  )
}
