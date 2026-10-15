---
title: faq
order: 60
summary: Des réponses courtes aux questions fréquentes, dont ce qui distingue Ethereal d’Ableton Live.
platform: all
audience: everyone
---

Des réponses rapides, chacune avec un lien vers la page qui en dit plus. Vous ne trouvez pas votre question ? Cherchez dans la documentation, ou [signalez un problème](/docs/reference/reporting-issues) pour la poser.

## les bases {#the-basics}

### dois-je utiliser l’application de bureau ou la version web ? {#should-i-use-the-desktop-app-or-the-web-version}

Les deux font de la musique avec les mêmes instruments et effets intégrés. L’application de bureau (macOS, Windows et Linux) ajoute les plug-ins (CLAP, VST3, AU), l’enregistrement, le MIDI matériel et l’apprentissage MIDI, et garde vos projets sous forme de fichiers sur votre ordinateur. La version web ne demande rien à installer, ce qui en fait un bon moyen d’essayer Ethereal ou de rejoindre la session d’un ami. Voir [qu’est-ce qu’Ethereal](/docs/welcome/what-is-ethereal).

### où sont mes projets ? {#where-are-my-projects}

Dans l’application de bureau, ce sont des dossiers sur votre ordinateur. Dans la version web, ils sont dans votre navigateur, sur cet ordinateur seulement, et ils peuvent être perdus si les données du navigateur sont effacées. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live).

### l’enregistrement est-il automatique ? {#does-it-save-automatically}

Oui. Ethereal enregistre pendant que vous travaillez, et garde des sauvegardes automatiques auxquelles vous pouvez revenir. Voir [projets et enregistrement](/docs/basics/projects) et [versions et sauvegardes](/docs/basics/versions-and-backups).

### puis-je déplacer un projet vers un autre ordinateur ? {#can-i-move-a-project-to-another-computer}

Oui : exportez-le depuis le menu du projet sous forme de fichier `.ether` et ouvrez ce fichier sur l’autre ordinateur, dans l’application de bureau ou la version web. Voir [déplacer des projets entre ordinateurs](/docs/basics/moving-projects).

### est-ce que ça marche sur un téléphone ? {#does-it-work-on-a-phone}

Une version tactile pour téléphones et tablettes est en bêta. Voir [Ethereal sur votre téléphone](/docs/mobile/mobile-beta).

### est-ce que ça marche hors ligne ? {#does-it-work-offline}

L’application de bureau fait de la musique sans connexion internet. Le partage, rejoindre des sessions, les nouveautés, la recherche de mises à jour, les signalements de problèmes et l’assistant IA en ont besoin.

## faire de la musique {#making-music}

### pourquoi mes plug-ins n’apparaissent-ils pas ? {#why-dont-my-plugins-show-up}

Les plug-ins demandent l’application de bureau. Dans l’application de bureau, vérifiez **réglages › plug-ins** et relancez l’analyse. Voir [plug-ins](/docs/sounds-and-plugins/plugins) et [dépannage](/docs/reference/troubleshooting#plugins-crash-or-fail-to-load).

### puis-je enregistrer ? {#can-i-record}

Oui, dans l’application de bureau : de l’audio depuis votre interface ou votre micro, et du MIDI depuis votre clavier. La version web ne peut pas enregistrer. Voir [enregistrement](/docs/making-music/recording).

### je n’ai pas de clavier midi. puis-je quand même jouer des notes ? {#i-dont-have-a-midi-keyboard-can-i-still-play-notes}

Oui : appuyez sur [[action:midi.computerKeyboard]] pour jouer des notes avec le clavier de votre ordinateur. Voir [le clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard). Vous pouvez aussi dessiner des notes dans [le piano roll](/docs/making-music/piano-roll).

### où sont tous les raccourcis ? {#where-are-all-the-shortcuts}

Dans [raccourcis clavier](/docs/reference/keyboard-shortcuts). Chaque commande est aussi dans la palette de commandes, [[action:palette.open]], et vous pouvez y modifier n’importe quel raccourci avec **raccourcis clavier…**.

## fichiers et autres applications {#files-and-other-apps}

### puis-je ouvrir mes projets ableton live ou fl studio ? {#can-i-open-my-ableton-live-or-fl-studio-projects}

Oui, en bêta : les sets Ableton Live (`.als`), les projets FL Studio (`.flp`), les fichiers DAWproject et les projets REAPER s’ouvrent comme un nouveau projet Ethereal. Un rapport liste ce qui n’a pas pu être repris, et vous aide à remplacer les plug-ins et échantillons manquants. Voir [projets d’autres DAW](/docs/import-export/other-daws).

### puis-je envoyer mon projet vers une autre daw ? {#can-i-send-my-project-to-another-daw}

Oui : exportez en DAWproject, en set Ableton Live (`.als`) ou en projet FL Studio (`.flp`). Voir [exporter vers d’autres DAW](/docs/import-export/exporting-to-other-daws).

### quels formats audio puis-je exporter ? {#which-audio-formats-can-i-export}

WAV, FLAC et MP3, en un seul mix ou en stems. Voir [exporter l’audio](/docs/import-export/exporting-audio).

## partage {#sharing}

### comment faire de la musique avec un ami ? {#how-do-i-make-music-with-a-friend}

Appuyez sur **partager** et envoyez-lui un lien. Il peut rejoindre depuis l’application de bureau ou directement depuis son navigateur, sans compte. Chacun joue la musique sur son propre ordinateur. Voir [partager votre musique](/docs/sharing/overview).

### si j’annule dans une session, est-ce que j’annule le travail de mon ami ? {#if-i-undo-in-a-session-do-i-undo-my-friends-work}

Non. Annuler n’annule que vos propres modifications. Voir [annuler et historique](/docs/basics/undo-and-history).

### mon projet partagé est-il privé ? {#is-my-shared-project-private}

Une session en direct circule chiffrée entre vos ordinateurs, et les personnes qui y participent peuvent voir l’adresse IP des autres. Toute personne qui a un lien peut l’utiliser : ne partagez donc vos liens qu’avec des personnes de confiance, et réinitialisez un lien que vous avez trop diffusé. Voir [sessions en direct](/docs/sharing/live-sessions) et [confidentialité et télémétrie](/docs/reference/privacy-and-telemetry).

## différences avec ableton live {#differences-from-ableton-live}

Ethereal vous semblera familier si vous connaissez Live, avec quelques différences bonnes à savoir :

- **Un seul arrangement, pas de vue session.** Il n’y a ni lanceur de clips ni scènes : vous arrangez sur une timeline. Voir [arrangement](/docs/making-music/arranging).
- **Copiez un clip en le faisant glisser avec [[key:Mod]].** Un glissement avec [[key:Alt]] ne copie pas : il désactive le magnétisme pendant le glissement. Voir [clips](/docs/making-music/clips).
- **Cliquer sur une piste place un marqueur d’insertion.** Coller, scinder et insérer se font à ce marqueur, pas à la tête de lecture. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).
- **Espace revient là où vous avez commencé.** [[action:transport.play]] lance la lecture et, appuyé de nouveau, arrête et revient en arrière. [[action:transport.stopToSelection]] arrête et laisse la tête de lecture où elle est. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).
- **Les raccourcis de Live sont à un réglage près.** Dans **raccourcis clavier…**, le preset **Ableton-like** ajoute les raccourcis de Live à ceux d’Ethereal, comme [[action:transport.record]] pour enregistrer, [[action:transport.loop]] pour la boucle et [[action:track.addAudio]] / [[action:track.addMidi]] pour de nouvelles pistes audio et MIDI. Rien n’est retiré. Voir [raccourcis clavier](/docs/reference/keyboard-shortcuts).
- **Le clavier MIDI de l’ordinateur est sur [[action:midi.computerKeyboard]]**, comme dans Live.
- **Pas de vue mixeur séparée.** Volume, panoramique, muet et solo sont sur les en-têtes de piste, et les envois sont dans l’inspecteur. Voir [mixage](/docs/making-music/mixing).
- **Deux modes de warp** : repitch et complex. Voir [clips audio](/docs/making-music/audio-clips).
- **Les plug-ins peuvent tourner isolés**, dans leur propre processus, pour qu’un plug-in qui plante n’entraîne pas l’application avec lui. Voir [plug-ins](/docs/sounds-and-plugins/plugins).
- **Les sessions en direct sont intégrées.** Partagez un lien et travaillez ensemble sur le même projet. Voir [partager votre musique](/docs/sharing/overview).
- **Les projets Live s’ouvrent dans Ethereal**, et Ethereal peut exporter des fichiers `.als`, en bêta. Voir [projets d’autres DAW](/docs/import-export/other-daws).

## et ensuite {#where-next}

- [Glossaire](/docs/reference/glossary).
- [Dépannage](/docs/reference/troubleshooting).
- [Votre première boucle en 5 minutes](/docs/welcome/first-beat).
