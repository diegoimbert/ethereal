// The Chat section of the left sidebar (docs/COLLAB.md §12.1): the project's chat journal
// in chat order, author names in their snapshot colours, and the input (Enter sends,
// Shift+Enter new line, a counter near the cap). Shown only in a session.
import "./social.css";
import clsx from "clsx";
import { useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import type { ChatMessage, SiteId } from "@/generated";
import { Button, MOD_KEY } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId, TransportContext } from "@/transport";
import { peerColor, useCollabStore } from "../store";
import { returnFocus, useChatUi } from "./chatStore";
import { CHAT_MAX_CHARS, charCount, chatOrdered, chatTextError, relativeTime } from "./chatText";

/** The counter shows from this many characters left. */
const COUNTER_FROM = 200;

/** Messages by the same author within this long are grouped under one header. */
const GROUP_MS = 5 * 60_000;

function useNow(periodMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), periodMs);
    return () => clearInterval(t);
  }, [periodMs]);
  return now;
}

function authorColor(m: ChatMessage): string {
  return m.author.color !== null ? peerColor(m.author.color) : "var(--eth-color-text-dim)";
}

export function ChatPanel() {
  const status = useCollabStore((s) => s.status);
  const chat = useProjectStore((s) => s.project?.chat);
  const messages = useMemo(() => chatOrdered(chat), [chat]);
  const me = status.type === "Online" ? status.site : null;
  if (status.type === "Offline") {
    return (
      <div className="eth-chat eth-chat--offline" data-chat-panel data-testid="chat-panel">
        <p className="eth-chat__hint">Chat is available in a collaboration session. Messages are saved with the project.</p>
      </div>
    );
  }
  return (
    <div className="eth-chat" data-chat-panel data-testid="chat-panel">
      <MessageList messages={messages} me={me} />
      <Composer disabled={status.type !== "Online"} />
    </div>
  );
}

function MessageList({ messages, me }: { messages: ChatMessage[]; me: SiteId | null }) {
  const now = useNow(30_000);
  const ref = useRef<HTMLOListElement>(null);
  const stick = useRef(true);
  // Stay at the bottom while the user hasn't scrolled up.
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages.length]);
  if (messages.length === 0) {
    return <p className="eth-chat__hint eth-chat__empty">No messages yet. Say hi, the chat is saved with the project.</p>;
  }
  return (
    <ol
      ref={ref}
      className="eth-chat__list"
      aria-label="Messages"
      data-testid="chat-messages"
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 8;
      }}
    >
      {messages.map((m, i) => {
        const prev = messages[i - 1];
        const grouped =
          !!prev && prev.author.site === m.author.site && prev.author.name === m.author.name && m.sent_at - prev.sent_at < GROUP_MS && m.sent_at >= prev.sent_at;
        const own = me !== null && m.author.site === me;
        return (
          <li
            key={m.id}
            className={clsx("eth-chat__message", grouped && "eth-chat__message--grouped", own && "eth-chat__message--own")}
            style={{ "--eth-chat-author": authorColor(m) } as CSSProperties}
            data-testid="chat-message"
            data-author={m.author.name}
          >
            {!grouped && (
              <div className="eth-chat__meta">
                <span className="eth-chat__author">{own ? "You" : m.author.name || "Someone"}</span>
                <time className="eth-chat__time" dateTime={new Date(m.sent_at).toISOString()} title={new Date(m.sent_at).toLocaleString()}>
                  {relativeTime(m.sent_at, now)}
                </time>
              </div>
            )}
            <p className="eth-chat__text">{m.text}</p>
          </li>
        );
      })}
    </ol>
  );
}

function Composer({ disabled }: { disabled: boolean }) {
  const transport = useContext(TransportContext)?.transport ?? null;
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const focusRequest = useChatUi((s) => s.focusRequest);
  const handled = useRef(0);
  useEffect(() => {
    if (focusRequest !== handled.current) {
      handled.current = focusRequest;
      input.current?.focus();
    }
  }, [focusRequest]);
  const left = CHAT_MAX_CHARS - charCount(text);
  const invalid = chatTextError(text);
  const send = () => {
    if (!transport || invalid) return;
    const body = text;
    setText("");
    setError(null);
    transport.send(cmd("Chat", { type: "Send", id: newId(), text: body })).catch((e: unknown) => {
      setText((t) => (t ? t : body));
      setError(e instanceof Error ? e.message : String(e));
    });
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      returnFocus();
    }
  };
  return (
    <div className="eth-chat__composer">
      <textarea
        ref={input}
        className="eth-input eth-chat__input"
        aria-label="Message"
        placeholder={`Message everyone (${MOD_KEY}⇧M)`}
        rows={2}
        value={text}
        disabled={disabled}
        aria-invalid={left < 0 || undefined}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        data-testid="chat-input"
      />
      <div className="eth-chat__footer">
        {error ? (
          <span className="eth-chat__error" role="alert">
            {error}
          </span>
        ) : (
          <span className="eth-chat__hint">Enter to send, Shift+Enter for a new line</span>
        )}
        {left <= COUNTER_FROM && (
          <span className={clsx("eth-chat__counter", left < 0 && "eth-chat__counter--over")} data-testid="chat-counter">
            {left}
          </span>
        )}
        <Button size="sm" tone="accent" disabled={disabled || !!invalid || !transport} onClick={send}>
          Send
        </Button>
      </div>
    </div>
  );
}
