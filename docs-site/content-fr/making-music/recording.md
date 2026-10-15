---
title: enregistrement
order: 140
summary: Armez des pistes, choisissez les entrées et le monitoring, utilisez le décompte et le punch, et enregistrez de l’audio et du MIDI.
platform: desktop
audience: beginner
covers:
  - recording/recording-needs-the-desktop-app #recording-needs-the-desktop-app
  - recording/arm-a-track-then-record #your-first-recording
  - recording/arming-is-exclusive-cmd-ctrl-click-arms-several #arming-several-tracks
  - recording/arm-state-isn-t-saved-with-the-project #arming-several-tracks
  - recording/arming-an-audio-track-with-no-input-device-opens-settings #your-first-recording
  - recording/the-in-panel-input-device-per-track-arm-input-and-monitor #the-in-panel
  - recording/monitor-modes-auto-in-off #monitoring
  - recording/midi-input-port-and-channel-filters #midi-input-filters
  - recording/count-in-pre-roll #count-in
  - recording/punch-records-only-inside-the-loop-region #punch-in-and-out
  - recording/recording-over-clips-or-in-a-loop-makes-takes-not-overdub #recording-over-clips-and-in-a-loop
  - recording/takes-show-lanes-swipe-to-comp-up-down-to-switch #recording-over-clips-and-in-a-loop
  - recording/live-view-while-recording #while-you-record
  - recording/silent-take-warning-macos-microphone-permission #a-silent-take
  - recording/switching-projects-while-recording-loses-the-recording #while-you-record
  - recording/recording-another-track-resampling #recording-another-track
---

Cette page explique comment enregistrer de l’audio (une voix, une guitare) et du MIDI (un clavier ou un contrôleur) sur des pistes. Il vous faut une piste audio ou une piste MIDI (voir [les pistes](/docs/making-music/tracks)). Pour l’audio, configurez d’abord votre périphérique d’entrée dans [configuration audio et MIDI](/docs/making-music/audio-and-midi-setup).

## l’enregistrement demande l’application de bureau [[web]] {#recording-needs-the-desktop-app}

Dans le navigateur, les boutons d’enregistrement et de **punch** sont désactivés, et le panneau des entrées indique que l’enregistrement demande l’application de bureau : le navigateur ne donne à Ethereal aucun accès aux entrées audio ou MIDI. Deux choses fonctionnent quand même sur le web :

- le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard), pour jouer des notes,
- [capturer le MIDI](/docs/making-music/capture-midi), pour transformer ce que vous venez de jouer en clip.

[Téléchargez l’application de bureau](/download) pour enregistrer de l’audio ou du MIDI matériel.

## votre premier enregistrement {#your-first-recording}

1. Cliquez sur **●** dans l’en-tête d’une piste pour l’**armer** : la piste est prête à enregistrer.
2. Appuyez sur le bouton d’enregistrement (**●**) dans la barre de transport. La lecture démarre (après le [décompte](#count-in), si vous en avez réglé un) et le transport affiche **rec**.
3. Jouez ou chantez.
4. Arrêtez, avec le bouton stop ou [[action:transport.play]].

Tout ce qui est enregistré devient des clips, en une seule étape de l’[historique d’annulation](/docs/basics/undo-and-history) : une seule annulation retire tout l’enregistrement.

Si vous armez une piste audio alors qu’aucun périphérique d’entrée n’est encore ouvert, les **réglages** s’ouvrent sur l’onglet **audio** et vous demandent de choisir un périphérique d’entrée. Sans lui, l’enregistrement serait muet.

L’enregistrement n’a pas de raccourci par défaut. Vous pouvez lui en attribuer un dans [raccourcis clavier](/docs/reference/keyboard-shortcuts). Le preset **ableton-like** utilise [[key:F9]].

### armer plusieurs pistes {#arming-several-tracks}

Un simple clic sur le **●** d’une piste arme cette piste et désarme toutes les autres. Pour enregistrer plusieurs pistes à la fois, faites [[key:Mod]]-clic sur leurs boutons **●** pour les ajouter. Dans le panneau **in ▾** (voir plus bas), les boutons d’armement ne désarment jamais les autres pistes.

Les pistes armées sont mémorisées sur cet ordinateur pour chaque projet : quand vous rouvrez un projet, les mêmes pistes sont de nouveau armées. L’armement n’est pas enregistré dans le fichier du projet (il ne voyage pas avec lui et n’atteint pas les participants d’une session en direct) et ne fait pas partie de l’historique d’annulation.

## le panneau in {#the-in-panel}

Le bouton **in ▾** à côté des commandes d’enregistrement ouvre le panneau des entrées. Il contient :

- **entrée audio** : le périphérique d’entrée (le même choix que dans les réglages audio).
- un tableau avec toutes les pistes audio et MIDI, en trois colonnes :
  - **armer** : arme la piste sans désarmer les autres.
  - **entrée** : ce que la piste enregistre. Pour une piste audio : **aucune entrée**, chaque canal mono de votre interface, ou une paire stéréo comme **1/2**. Pour une piste MIDI : **tous les ports** ou un seul port MIDI, plus un filtre de canal.
  - **monitoring** : voir plus bas.

### monitoring {#monitoring}

Le monitoring consiste à entendre l’entrée à travers la piste (et ses effets) pendant que vous jouez.

| mode | ce que vous entendez |
|---|---|
| **auto** (par défaut) | l’entrée tant que la piste est armée |
| **in** | l’entrée en permanence, armée ou non |
| **off** | jamais l’entrée |

> [!warning]
> Écouter un micro à travers des enceintes peut provoquer un larsen, un sifflement puissant. Utilisez un casque, ou réglez le monitoring de la piste sur **off** et écoutez la source directement.

### filtres d’entrée midi {#midi-input-filters}

L’entrée d’une piste MIDI est par défaut **tous les ports**, avec **tous les canaux**. Chaque piste MIDI armée enregistre tout ce que son filtre accepte : avec les réglages par défaut, chaque piste MIDI armée enregistre donc tous les contrôleurs.

Pour enregistrer deux contrôleurs (ou deux canaux MIDI) sur deux pistes à la fois, réglez l’entrée de chaque piste sur un seul port, et choisissez un canal (**canal 1** à **canal 16**) si besoin.

## décompte {#count-in}

Le sélecteur de **décompte** (désactivé, 1, 2 ou 4 mesures) lance la lecture autant de mesures avant la position d’enregistrement, pour vous laisser le temps de vous préparer. Le réglage est enregistré avec le projet. Rien de ce que vous jouez pendant le décompte n’est conservé : une note jouée trop tôt est perdue.

## punch in et out {#punch-in-and-out}

Avec **punch** activé, seul ce que vous jouez dans la zone de boucle est conservé. L’accolade de boucle sur la règle définit la plage de punch : il n’y a pas de marqueurs de punch séparés. Chaque passage dans la boucle devient une [prise](/docs/making-music/takes-and-comping).

Le punch est mémorisé sur cet ordinateur pour chaque projet, comme les pistes armées : il est de nouveau activé quand vous rouvrez un projet que vous aviez quitté avec le punch activé.

## enregistrer sur des clips et en boucle {#recording-over-clips-and-in-a-loop}

L’enregistrement ne mélange jamais de nouveau contenu dans un clip existant, et il n’y a pas de mode d’overdub MIDI. À la place, des prises sont créées :

- quand vous enregistrez sur des clips existants,
- quand vous enregistrez plusieurs passages de la boucle,
- quand vous utilisez le punch.

Chaque passage devient une prise, sur son propre couloir de prises, et la plus récente est lue. Les clips existants dans la plage enregistrée passent sur le premier couloir de prises, ils ne sont donc pas perdus. Cliquez sur le bouton des prises dans l’en-tête de la piste pour afficher les couloirs de prises. Voir [prises et comping](/docs/making-music/takes-and-comping) pour choisir les meilleurs passages.

> [!note]
> Si vous venez d’Ableton Live et attendez l’overdub MIDI, enregistrez un nouveau passage puis faites un comping, ou enregistrez sur une autre piste.

## pendant l’enregistrement {#while-you-record}

Pendant l’enregistrement, un clip « recording » qui s’allonge affiche la forme d’onde ou les notes au fur et à mesure, et les notes tenues s’allongent tant que vous les tenez. Une forme d’onde plate est le premier signe d’une mauvaise entrée ou d’un mauvais canal. À l’arrêt, il est remplacé par les vrais clips au même endroit.

Si vous ouvrez ou créez un autre projet pendant l’enregistrement, l’enregistrement s’arrête d’abord et les prises vont dans le projet où vous enregistriez, qui est sauvegardé avant que l’autre s’ouvre.

### une prise muette {#a-silent-take}

Si une prise est complètement muette pendant au moins une seconde, un avertissement apparaît. Sur macOS, la cause la plus fréquente est que le système a refusé à Ethereal l’accès au micro : autorisez-le dans **Réglages Système › Confidentialité et sécurité › Microphone**. Vérifiez aussi le périphérique d’entrée et les canaux d’entrée de la piste.

## enregistrer une autre piste {#recording-another-track}

Une piste audio peut enregistrer la sortie d’une autre piste, par exemple pour rééchantillonner un synthé avec ses effets. Sélectionnez la piste, puis dans l’inspecteur réglez **entrée** sur **depuis** l’autre piste. Un second sélecteur choisit où le signal est prélevé : **pré-fx**, **post-fx** ou **post-mixeur**. Choisir une piste règle aussi le monitoring de la piste qui enregistre sur **in**, pour que vous l’entendiez.

L’enregistrement est aligné sur la latence propre de cette piste. Cela ne fonctionne que dans l’application de bureau. Dans le navigateur, armer une telle piste est désactivé.

## et ensuite {#where-next}

- [Prises et comping](/docs/making-music/takes-and-comping) : choisir les meilleurs passages de plusieurs prises.
- [Capturer le MIDI](/docs/making-music/capture-midi) : garder ce que vous avez joué sans enregistrer.
- [Clips audio](/docs/making-music/audio-clips) : fondus, warp et inversion sur ce que vous avez enregistré.
