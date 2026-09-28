/** MockTransport: chat and pinned notes (collab-social, docs/COLLAB.md §12). */
import { describe, expect, it } from "vitest";
import type { NotePosition } from "@/generated";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import type { MockCollab } from "./collab";
import { MOCK_OWN_COLOR, MOCK_SITE } from "./collab";
import { CHAT_MAX_MESSAGES, chatOrdered, insertChat } from "./social";
import { project, undo, useMock, type MockFixture } from "./testUtils";
import { Tx } from "../tx";

const at = (beats: number): NotePosition => ({ beats, track: null, y: 0 });
const join = (f: MockFixture) => f.mock.send(cmd("Collab", { type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Ada" }));
const sim = (f: MockFixture) => (f.mock as unknown as { collab: MockCollab }).collab;
const texts = (f: MockFixture) => chatOrdered(Object.values(project(f).chat)).map((m) => m.text);

describe("MockTransport social", () => {
  const f = useMock();

  it("chat needs a session and is validated", async () => {
    await expect(f.mock.send(cmd("Chat", { type: "Send", id: newId(), text: "hi" }))).rejects.toMatchObject({ code: "InvalidState" });
    await join(f);
    await expect(f.mock.send(cmd("Chat", { type: "Send", id: newId(), text: "  " }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(f.mock.send(cmd("Chat", { type: "Send", id: newId(), text: "x".repeat(2001) }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    expect(project(f).chat).toEqual({});
  });

  it("sends in order with the session identity, outside the undo history", async () => {
    await join(f);
    await f.mock.send(cmd("Marker", { type: "Add", id: newId(), position: 4, name: null, color: null }));
    const id = newId();
    await f.mock.send(cmd("Chat", { type: "Send", id, text: "first" }));
    await f.mock.send(cmd("Chat", { type: "Send", id, text: "resend" }));
    await f.mock.send(cmd("Chat", { type: "Send", id: newId(), text: "second" }));
    expect(texts(f)).toEqual(["first", "second"]);
    expect(project(f).chat[id]!.author).toEqual({ name: "Ada", site: MOCK_SITE, actor: null, color: MOCK_OWN_COLOR });
    // Undo removes the marker, not the messages.
    await undo(f);
    expect(project(f).markers).toEqual({});
    expect(texts(f)).toEqual(["first", "second"]);
  });

  it("prunes the oldest past the cap", () => {
    const p = project(f);
    const tx = new Tx(p);
    for (let i = 0; i < CHAT_MAX_MESSAGES + 1; i++) {
      insertChat(tx, { id: `m${String(i).padStart(5, "0")}`, author: { name: "", site: "2", actor: null, color: null }, text: `m${i}`, sent_at: 0 });
    }
    const ordered = chatOrdered(tx.all("ChatMessage"));
    expect(ordered).toHaveLength(CHAT_MAX_MESSAGES);
    expect(ordered[0]!.text).toBe("m1");
  });

  it("simulates a peer's message and transport", async () => {
    await join(f);
    f.events.length = 0;
    const id = sim(f).simulateChat("2", "hello from the mock peer")!;
    expect(project(f).chat[id]!.author.name).toBe("Mock peer");
    expect(f.events).toContainEqual({ type: "Collab", event: { type: "ChatReceived", ids: [id] } });
    sim(f).simulatePeerTransport("2", { position: 4, playing: true, sent_at_ms: 1, loop_region: null });
    const presence = f.events.findLast((e) => e.type === "Collab" && e.event.type === "Presence");
    expect(presence).toMatchObject({ event: { peers: [{ site: "2", state: { transport: { position: 4, playing: true } } }] } });
  });

  it("pinned notes are undoable document edits with an author", async () => {
    const id = newId();
    await f.mock.send(cmd("PinnedNote", { type: "Add", id, position: at(8), text: "look here", author_name: " Diego " }));
    expect(project(f).pinned_notes[id]).toMatchObject({ text: "look here", resolved: false, author: { name: "Diego", site: null, color: null } });
    await f.mock.send(cmd("PinnedNote", { type: "Edit", id, text: null, position: at(9), resolved: true }));
    expect(project(f).pinned_notes[id]).toMatchObject({ position: { beats: 9 }, resolved: true });
    await f.mock.send(cmd("PinnedNote", { type: "Delete", ids: [id] }));
    expect(project(f).pinned_notes[id]).toBeUndefined();
    await undo(f);
    expect(project(f).pinned_notes[id]).toMatchObject({ resolved: true });
    await expect(f.mock.send(cmd("PinnedNote", { type: "Add", id: newId(), position: at(-1), text: "x", author_name: null }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    // In a session: the session identity.
    await join(f);
    const mine = newId();
    await f.mock.send(cmd("PinnedNote", { type: "Add", id: mine, position: at(1), text: "mine", author_name: "ignored" }));
    expect(project(f).pinned_notes[mine]!.author).toMatchObject({ name: "Ada", site: MOCK_SITE });
  });
});
