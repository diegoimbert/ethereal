---
title: audio vers midi
order: 60
summary: Transformez une mélodie, des accords ou une boucle de batterie d’un clip audio en notes MIDI sur une nouvelle piste.
platform: all
audience: advanced
covers:
  - midi-editing/convert-audio-to-midi #converting-a-clip
  - midi-editing/audio-to-midi-one-conversion-at-a-time-main-lane-clips-only #one-at-a-time-main-lane-clips-only
---

La conversion audio vers MIDI écoute un clip audio et écrit les notes qu’elle entend dans un nouveau clip MIDI. Vous pouvez ensuite les jouer avec n’importe quel instrument, les corriger dans le piano roll, ou doubler une partie.

## convertir un clip {#converting-a-clip}

1. Faites un clic droit sur un clip audio et choisissez **convertir en MIDI…**.
2. Choisissez ce que contient le clip :
   - **mélodie** : une note à la fois (voix, lignes de basse, leads) ;
   - **harmonie** : des accords et plusieurs notes à la fois (piano, guitare, nappes) ;
   - **batterie** : les coups de grosse caisse, de caisse claire et de charleston.
3. Ajustez les réglages si besoin : la **sensibilité**, et pour la mélodie et l’harmonie la **note la plus courte** et la **tessiture** ; pour la batterie, les touches **grosse caisse**, **caisse claire** et **charleston** sur lesquelles vont les coups. **réinitialiser les réglages** remet les valeurs par défaut.
4. Si vous le souhaitez, activez **ajouter un synthé polyphonique** (ou, pour la batterie, **ajouter un drum rack avec un kit**) pour que la nouvelle piste joue tout de suite.
5. Cliquez sur **convertir**.

Le résultat est une nouvelle piste MIDI juste sous la piste source, avec un clip à la même position et de la même longueur. Toute la conversion compte comme une seule étape d’annulation.

## une à la fois, clips du couloir principal seulement {#one-at-a-time-main-lane-clips-only}

- Une seule conversion tourne à la fois. Pendant qu’elle tourne, **convertir en MIDI…** est grisé sur les autres clips.
- **masquer** ferme la fenêtre et poursuit la conversion : une barre de progression défile en bas du clip. Cliquez dessus pour l’arrêter, ou choisissez **arrêter la conversion en MIDI** dans le menu du clic droit du clip.
- Seuls les clips du couloir principal d’une piste peuvent être convertis, pas ceux des [couloirs de prises](/docs/making-music/takes-and-comping).
- Si une conversion échoue, le clip l’indique : cliquez sur la marque pour voir les détails.

## et ensuite {#where-next}

- [le piano roll](/docs/making-music/piano-roll) : nettoyez les notes.
- [le drum rack](/docs/making-music/drum-rack)
- [groove, swing et humanisation](/docs/advanced/groove)
