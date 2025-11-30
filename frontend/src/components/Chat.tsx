import { useState, useRef, useEffect } from 'react'
import { chatStream } from '../api'
import './Chat.css'

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
      <div className="chat-container">
        <div className="chat-header">
          <h3>💬 Quick Chat</h3>
          <p className="chat-caption">Powered by Qwen3-4B</p>
        </div>
        <div className="chat-messages" style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          minHeight: '300px',
          padding: '2rem'
        }}>
          <div style={{
            textAlign: 'center',
            color: 'var(--text-muted)',
            maxWidth: '400px'
          }}>
            <div style={{ fontSize: '3rem', marginBottom: '1rem' }}>🔒</div>
            <h4>Chat Disabled</h4>
            <p style={{ marginTop: '0.5rem', lineHeight: '1.6' }}>
              The local LLM is currently disabled. Enable it in Settings to use the chat feature.
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="chat-container">
      <div className="chat-header">
        <h3>💬 Quick Chat</h3>
        <p className="chat-caption">Powered by Qwen3-4B · Does not use indexed context yet</p>
      </div>

      <div className="chat-messages">
        {messages.map((msg, idx) => (
          <div key={idx} className={`chat-message ${msg.role}`}>
            <div className="message-content">
              {msg.content || (isLoading && idx === messages.length - 1 ? '...' : '')}
            </div>
          </div>
        ))}
        <div ref={messagesEndRef} />
      </div>

      <form onSubmit={handleSubmit} className="chat-input-form">
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Type a message..."
          disabled={isLoading}
        />
        <button type="submit" disabled={isLoading || !input.trim()}>
          Send
        </button>
      </form>
    </div>
  )
}
