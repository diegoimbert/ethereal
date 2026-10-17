---
title: the ai assistant
order: 10
summary: Ask an AI model to edit your project, with your own API key or a model on your computer.
platform: all
audience: advanced
covers:
  - ai-mcp/ask-ai-panel #the-ask-ai-panel
  - ai-mcp/bring-your-own-api-key #connecting-a-provider
  - ai-mcp/where-your-key-is-stored #where-your-key-is-stored
  - ai-mcp/what-is-sent-to-the-ai-provider #what-is-sent-to-the-provider
  - ai-mcp/each-ai-action-is-one-undo-step #every-ai-edit-is-one-undo-step
  - ai-mcp/tool-calls-stop-and-round-limit #tool-calls-stop-and-the-round-limit
  - ai-mcp/switching-provider-starts-fresh #the-saved-conversation-and-switching-provider
  - ai-mcp/web-cors-blocks-some-providers #providers-blocked-in-the-browser
  - ai-mcp/models-without-tool-support #models-without-tool-support
  - ai-mcp/what-the-ai-can-t-do #what-the-ai-can-and-cant-do
---

Ethereal has a chat panel where you describe an edit in plain words ("add a C minor arpeggio on a new track", "set the tempo to 92") and an AI model makes it in your open project. You need an API key from an AI provider, or a model running on your own computer. This page covers setting it up, what the AI sees, and how to undo what it does.

> [!note]
> The AI assistant is in **alpha**: it can make mistakes and is limited for now, and it will be heavily improved in the future. The panel shows an **alpha** badge, and says so in the first-use card and above your conversations (you can dismiss that line; the badge stays).

## the ask ai panel

Open the **ask ai** tab in the left rail, press [[action:ai:ask]], or run **ask ai** from the command palette ([[action:palette.open]]). Type a request and press [[key:Enter]] to send it ([[key:Shift+Enter]] adds a new line). An empty chat suggests a few requests to try.

The AI can create and rename tracks, write MIDI notes, add devices and set their parameters, set volume and pan, change the tempo, control the transport and export audio. You watch the edits appear in the arrangement as it works. A project has to be open first.

## connecting a provider

The first time you open the panel, it asks you to pick a provider and paste a key:

- Cloud providers: Anthropic (Claude, the default), OpenAI, DeepSeek, Google Gemini, Mistral, Groq, OpenRouter and Together AI.
- Local models: Ollama and LM Studio, running on your computer. They need no key.
- **other (openai-compatible)**: any server that speaks the OpenAI chat API.

You use your own key, and the provider bills you for what you use. Ethereal doesn't resell access. To change the provider, model, base URL or key later, click the **ai settings** button (the gear) in the panel's header. **test connection** checks the key and model with a free request. The model picker in the panel's header switches models quickly.

### where your key is stored

- [[desktop]] In Ethereal's app data, on this computer.
- [[web]] In this browser's local storage. Anyone who uses the same browser profile can read it.

Either way, the key is only ever sent to the provider you chose. It isn't sent to the engine or to Ethereal's servers.

## what is sent to the provider

Each message goes to the provider together with an overview of your project: track, clip, device and marker names, the mix and the tempo. Whatever the model reads with its tools (notes, parameter values) is sent too. Audio files are never sent.

The first time you use a provider, the panel says what it sends (project contents: tracks, clips, notes, device settings) and to whom, and waits for you to agree with **send to …**. It asks once per provider and remembers your answer on this computer. Switching to another provider asks again.

> [!warning]
> Once you agree, every message sends your project's names and contents to that provider. Don't use it on a project you can't share with that provider, or use a local model.

## every ai edit is one undo step

Each change the AI makes is applied right away as one undo step, named **ai: …** in the history (for example **ai: add notes**). Press [[action:edit.undo]] to undo the last one, or jump back in the **history** panel.

## tool calls, stop and the round limit

Each action the AI takes shows as a row in the chat. Expand it to see what the AI sent and what came back, errors included.

- **stop** (or [[key:Escape]] in the message field) interrupts the AI. The edits it already made stay, and you can undo them.
- A reply pauses after 25 rounds of tool calls, so a runaway request can't go on forever. Say "continue" to let it go on. Change the limit (1 to 200) under **tool rounds per message** in the AI settings.

## the saved conversation and switching provider

The conversation belongs to the project. It's kept on this computer (not in the project file, so it doesn't travel with the project) and comes back when you reopen the project or reload the app, and the AI picks up where it left off. Each project has its own conversation.

**clear chat** (the eraser button) empties the conversation and forgets the saved one. A very long conversation keeps only its latest part.

If you switch to another provider in the middle of a conversation, the next message starts a fresh conversation with the new provider. The panel says so, because the new model doesn't see the messages above.

## providers blocked in the browser [[web]]

In the web version, some providers refuse requests that come straight from a web page (CORS). When that happens, the panel says the provider blocked the request and suggests a fix:

- Ollama: allow the page's origin with the `OLLAMA_ORIGINS` setting.
- LM Studio: turn on **enable cors** in its server settings.
- Cloud APIs: use OpenRouter, or a proxy.

The desktop app isn't affected: it makes these requests itself, outside the browser.

## models without tool support

The AI edits your project by calling tools, so the model must support tool calls. Some local models don't. When the server says so, the panel tells you and suggests picking another model.

## what the ai can and can't do

The AI works only through Ethereal's own tools: the same edits you can make in the app. It can't read or write other files on your computer, run programs or open other projects.

- Plugins are desktop only, so in the web version the AI finds no plugins to add.
- Searching the sound library needs the library's index. Without it, the AI can't load samples or presets.

To let an AI app outside Ethereal edit your project, see [ai agents (mcp)](/docs/advanced/mcp).

## where next

- [ai agents (mcp)](/docs/advanced/mcp): connect Claude Code or Claude Desktop to the desktop app.
- [undo and history](/docs/basics/undo-and-history): step back through AI edits.
