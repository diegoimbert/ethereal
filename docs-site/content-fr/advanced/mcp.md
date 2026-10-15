---
title: agents ia (mcp)
order: 20
summary: Laissez Claude Code, Claude Desktop ou un autre client MCP modifier votre projet dans l’application de bureau, ou un fichier de projet sans aucune application.
platform: desktop
audience: advanced
covers:
  - ai-mcp/external-ai-agents-mcp #allow-external-ai-agents
  - ai-mcp/mcp-headless-and-remote-modes #without-the-app-project-files-and-engine-servers
---

En plus de l’[assistant IA](/docs/advanced/ai-assistant) intégré, les applications d’IA de votre ordinateur peuvent piloter Ethereal via MCP (le Model Context Protocol). Claude Code, Claude Desktop et d’autres clients MCP peuvent alors créer des pistes, écrire du MIDI, ajouter des appareils, mixer, lire et exporter, en direct dans votre projet ouvert. Cela fonctionne avec l’application de bureau. L’application web ne le permet pas.

## autoriser les agents ia externes {#allow-external-ai-agents}

C’est désactivé par défaut. Pour l’activer :

1. Ouvrez l’onglet **demander à l’IA** dans la barre latérale gauche et cliquez sur **réglages de l’IA** (la roue dentée).
2. Activez **autoriser les agents IA externes (MCP)**.

La section affiche alors le port local qu’elle écoute, le nombre d’agents connectés, et une commande `claude mcp add` avec un bouton pour la copier.

Par sécurité, elle n’accepte que les connexions venant de cet ordinateur (jamais du réseau), et chaque fois que vous l’activez, elle crée un nouveau jeton secret que seul votre compte utilisateur peut lire. La désactiver, ou quitter Ethereal, déconnecte tous les agents.

> [!warning]
> Un agent peut faire tout ce que vous pouvez faire dans le projet ouvert : le modifier, le lire, l’enregistrer et exporter dans le dossier du projet. Il ne peut pas ouvrir d’autres projets ni toucher à d’autres fichiers. Ne connectez que des agents de confiance.

## connecter claude code {#connecting-claude-code}

L’application de bureau installe le serveur MCP, `ether-mcp`, à côté de l’application. Une fois le réglage activé, copiez la commande depuis les réglages de l’IA et lancez-la dans un terminal. Elle ressemble à :

```
claude mcp add ethereal -- /Applications/Ethereal.app/Contents/MacOS/ether-mcp
```

Emplacement de `ether-mcp` :

| installation | `ether-mcp` |
|---|---|
| macOS | `/Applications/Ethereal.app/Contents/MacOS/ether-mcp` |
| Windows (installateur, par utilisateur) | `%LOCALAPPDATA%\Ethereal\ether-mcp.exe` |
| Windows (.msi) | `C:\Program Files\Ethereal\ether-mcp.exe` |
| Linux (.deb, .rpm) | `/usr/bin/ether-mcp` |

Avec l’AppImage Linux, le programme se trouve dans l’image, donc l’application affiche la commande sans chemin. Extrayez-le une fois avec `./Ethereal_*.AppImage --appimage-extract usr/bin/ether-mcp` et utilisez le fichier extrait.

Pour Claude Desktop, ajoutez le même chemin comme serveur MCP dans son fichier de configuration (Settings > Developer > Edit Config).

Demandez ensuite une modification à l’agent, par exemple « fais un motif de batterie de 4 mesures sur une nouvelle piste ». Chaque appel d’outil compte pour une étape d’annulation, que vous pouvez annuler dans Ethereal avec [[action:edit.undo]].

Si Ethereal est fermé ou si le réglage est désactivé, les appels de l’agent échouent avec un message indiquant qu’Ethereal est injoignable. Activez le réglage (ou lancez l’application) et réessayez : inutile de redémarrer l’agent.

## sans l’application : fichiers de projet et serveurs de moteur {#without-the-app-project-files-and-engine-servers}

`ether-mcp` a deux autres modes, pour les scripts et l’automatisation :

- `ether-mcp --project song.ether` ouvre directement un fichier de projet, sans application et sans son. Un chemin qui n’existe pas encore crée un nouveau projet. L’enregistrement écrit le fichier.
- `ether-mcp --server ws://host:port --token TOKEN` pilote un [serveur de moteur](/docs/advanced/engine-server).

## et ensuite {#where-next}

- [l’assistant ia](/docs/advanced/ai-assistant) : le chat intégré à Ethereal.
- [le serveur de moteur](/docs/advanced/engine-server) : faites tourner le moteur sur une autre machine.
- [annuler et historique](/docs/basics/undo-and-history)
