---
title: le clavier midi de l’ordinateur
order: 40
summary: Jouez et enregistrez des notes avec le clavier de votre ordinateur, sans matériel MIDI.
platform: all
audience: beginner
covers:
  - midi-editing/computer-midi-keyboard-m #turning-it-on
  - midi-editing/while-the-midi-keyboard-is-on-letter-shortcuts-stop-working #shortcuts-while-it-is-on
  - midi-editing/the-midi-keyboard-auto-arms-a-track #which-track-it-plays
  - midi-editing/the-midi-keyboard-keeps-playing-inside-plugin-windows #inside-plugin-windows
---

Pas besoin de contrôleur MIDI pour jouer des notes. Le clavier MIDI de l’ordinateur transforme les touches de lettres de votre ordinateur en petit piano. Il fonctionne dans l’application de bureau et sur le web, où c’est le seul moyen de jouer des notes en direct.

Il vous faut une piste MIDI avec un instrument. Voir [les pistes](/docs/making-music/tracks) et [les instruments](/docs/making-music/instruments).

## l’activer {#turning-it-on}

Appuyez sur [[action:midi.computerKeyboard]], ou cliquez sur le bouton piano dans la barre de transport. Tant qu’il est activé, le bouton est allumé et affiche à côté l’octave et la vélocité actuelles. Appuyez à nouveau sur la touche pour le désactiver.

Il reste activé ou désactivé d’une session à l’autre : si vous le laissez activé, il l’est encore la prochaine fois que vous ouvrez Ethereal.

## les touches {#the-keys}

La disposition est la même que dans Ableton Live. Elle utilise la position des touches, pas les lettres, et fonctionne donc de la même façon sur les claviers QWERTY, AZERTY et QWERTZ.

| touches | ce qu’elles font |
|---|---|
| la rangée du milieu, **A S D F G H J K L ;** | touches blanches, à partir de do |
| la rangée du dessus, **W E T Y U O P** | touches noires |
| **Z** / **X** | une octave plus bas / plus haut |
| **C** / **V** | vélocité plus basse / plus haute, par pas de 20 |

Il démarre à l’octave 3 (C3 est la note MIDI 60) avec une vélocité de 100. L’octave et la vélocité sont aussi mémorisées.

## quelle piste il fait jouer {#which-track-it-plays}

Le clavier se comporte exactement comme une entrée MIDI matérielle. Chaque piste MIDI qui écoute l’entrée en direct l’entend, et chaque piste MIDI armée l’enregistre. [Capturer le MIDI](/docs/making-music/capture-midi) garde aussi ce que vous jouez.

Si aucune piste MIDI ne l’entendrait, activer le clavier (ou jouer une note) arme pour vous la piste MIDI sélectionnée, ou la première piste MIDI si aucune n’est sélectionnée. Ableton fait de même. Survolez le bouton piano pour voir sur quelle piste il joue. Si le projet n’a pas de piste MIDI, l’infobulle vous invite à en ajouter une.

> [!note]
> À cause de cet armement automatique, une piste peut être armée sans que vous ayez cliqué sur son bouton d’enregistrement. La prochaine fois que vous enregistrez, vérifiez quelles pistes sont armées.

## les raccourcis quand il est activé {#shortcuts-while-it-is-on}

Tant que le clavier est activé, ses touches (les touches de notes et Z, X, C, V) jouent des notes au lieu de lancer le raccourci qui leur est attribué. Les raccourcis avec [[key:Mod]], [[key:Ctrl]], [[key:Alt]] ou [[key:Shift]] fonctionnent toujours, tout comme [[action:transport.play]]. La saisie dans un champ de texte n’est jamais affectée.

Avec les raccourcis par défaut, aucun raccourci à une seule lettre (comme [[action:pianoRoll.drawMode]] pour le mode dessin ou [[action:clip.reverse]] pour inverser) n’utilise ces touches. Si vous leur attribuez vos propres raccourcis, ils ne fonctionneront pas tant que le clavier est activé.

### dans les fenêtres de plug-in [[desktop]] {#inside-plugin-windows}

Dans l’application de bureau, la fenêtre d’édition d’un plug-in transmet ces touches à Ethereal tant que le clavier est activé. Vous pouvez continuer à jouer d’un synthé pendant que vous le réglez dans sa fenêtre. Quand le clavier est désactivé, la fenêtre du plug-in reçoit votre saisie comme d’habitude.

## et ensuite {#where-next}

- [Le piano roll](/docs/making-music/piano-roll) : modifier les notes que vous avez enregistrées.
- [Enregistrement](/docs/making-music/recording) : enregistrer ce que vous jouez.
- [Capturer le MIDI](/docs/making-music/capture-midi) : garder quelque chose que vous avez joué sans enregistrer.
