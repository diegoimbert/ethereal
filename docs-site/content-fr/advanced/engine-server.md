---
title: le serveur de moteur
order: 30
summary: Faites tourner le moteur audio d’Ethereal sur une autre machine et pilotez-le depuis cette fenêtre.
platform: all
audience: advanced
covers:
  - remote-engine/engine-server-remote-engine #connecting-to-an-engine-server
  - remote-engine/what-runs-where-with-an-engine-server #what-runs-where
  - remote-engine/engine-server-security #security
  - remote-engine/uploading-audio-to-an-engine-server #adding-audio-files
  - remote-engine/remote-engine-latency #latency-when-the-engine-is-elsewhere
  - remote-engine/audio-device-released-with-no-engine #when-your-own-engine-is-off
---

Normalement, le moteur audio (la partie qui lit et traite le son) tourne dans l’application ou dans l’onglet du navigateur. Un serveur de moteur, `ether-server`, est ce même moteur qui tourne seul, sans fenêtre, sur un autre ordinateur : par exemple une machine de studio avec vos plug-ins et votre interface audio. Cette fenêtre en devient alors la télécommande. C’est une configuration avancée : la plupart des gens n’en ont jamais besoin.

## se connecter à un serveur de moteur {#connecting-to-an-engine-server}

`ether-server` est un programme séparé. Il ne fait pas encore partie du téléchargement, vous devez donc le compiler à partir du code source d’Ethereal. Au démarrage, il affiche son adresse et l’endroit où trouver son jeton.

Comme le serveur ne fait pas partie du téléchargement, ses réglages restent masqués tant que vous ne les demandez pas :

1. Ouvrez la [palette de commandes](/docs/basics/command-palette-and-settings) et lancez **serveur de moteur…**. Dans l’application web, vous pouvez aussi ajouter `?engine-server` à l’adresse. Cela ouvre les **réglages** sur l’onglet **avancé**, où la section **serveur de moteur** reste désormais visible.
2. Sous **serveur de moteur**, saisissez l’**adresse** (`ws://host:port`) et le **jeton**.
3. Cliquez sur **se connecter**.

Pendant la connexion, la barre du haut affiche **●** et le nom du serveur. Cliquez dessus pour voir la version du serveur, ou pour **se déconnecter**. La déconnexion revient au moteur de cette fenêtre. Si la connexion tombe, la fenêtre revient d’elle-même à son propre moteur et indique que la connexion a été perdue.

La fenêtre retient l’adresse, mais jamais le jeton.

## ce qui tourne où {#what-runs-where}

Avec un serveur de moteur, le projet, les plug-ins et tout le traitement audio se trouvent sur le serveur. Le son sort du périphérique audio **du serveur**, pas des haut-parleurs de votre ordinateur. Votre fenêtre ne fait qu’afficher le projet et envoyer vos modifications.

Plusieurs fenêtres peuvent se connecter au même serveur en même temps. Elles partagent un seul projet et voient en direct les modifications des autres.

## ajouter des fichiers audio {#adding-audio-files}

Vos fichiers audio doivent parvenir au serveur. Pendant la connexion, déposez des fichiers audio sur le navigateur : ils sont envoyés au serveur et importés dans le projet. La barre d’état (en bas) affiche la progression de chaque envoi. Si vous vous déconnectez avant la fin d’un envoi, cet envoi est perdu.

## sécurité {#security}

- Par défaut, le serveur n’écoute que sur sa propre machine (`127.0.0.1`). Pour accepter d’autres machines, il doit être lancé avec `--listen`, et il exige alors un jeton.
- Sur tout réseau qui n’est pas de confiance, placez le serveur derrière TLS (un proxy inverse).

Quand une connexion est refusée, les réglages en donnent la raison :

| message | ce que cela signifie |
|---|---|
| jeton incorrect ou manquant | vérifiez le jeton affiché par le serveur. |
| ce serveur utilise une version incompatible d’ethereal | le serveur et cette fenêtre doivent avoir la même version. |
| le serveur n’accepte plus de clients pour le moment | le serveur est occupé : réessayez plus tard. |

## la latence quand le moteur est ailleurs {#latency-when-the-engine-is-elsewhere}

Quand vous écoutez un moteur sur un autre ordinateur (un serveur de moteur), chaque lecture ou arrêt fait un aller-retour sur le réseau, plus le temps d’encoder et de mettre en tampon l’audio. Cela représente quelques dizaines de millisecondes sur un réseau local, et davantage sur Internet. C’est suffisant pour la lecture et le mixage, pas pour jouer d’un instrument en direct.

Le MIDI en direct, le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) et l’enregistrement passent toujours par votre propre moteur, pas par le moteur distant.

## quand votre propre moteur est éteint [[desktop]] {#when-your-own-engine-is-off}

Quand votre propre moteur est éteint (par exemple pendant que vous utilisez un serveur de moteur), l’application de bureau ferme votre périphérique audio pour que d’autres applications puissent l’utiliser. L’état du moteur dans la barre de transport indique alors **fermé**, et les réglages audio le signalent aussi. Le périphérique se rouvre dès que votre moteur est rallumé.

## et ensuite {#where-next}

- [agents ia (mcp)](/docs/advanced/mcp) : `ether-mcp` peut aussi piloter un serveur de moteur.
- [configuration audio et midi](/docs/making-music/audio-and-midi-setup)
