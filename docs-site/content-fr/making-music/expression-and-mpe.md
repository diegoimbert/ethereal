---
title: expression et mpe
order: 170
summary: Dessinez le pitch bend, la molette de modulation, le sustain et d’autres contrôleurs dans un clip, et utilisez l’expression par note avec les contrôleurs MPE.
platform: all
audience: advanced
covers:
  - midi-editing/expression-lanes-cc-pitch-bend-pressure #choosing-a-lane
  - midi-editing/drawing-and-editing-expression-curves #drawing-and-editing-curves
  - midi-editing/per-note-lanes-draw-over-the-selected-notes-only #per-note-lanes
  - midi-editing/mpe-turn-it-on-per-track-in-the-inspector #mpe
---

Un clip MIDI ne contient pas que des notes. L’expression, c’est le mouvement entre les notes et à l’intérieur : le pitch bend, la molette de modulation, la pédale de sustain, la pression. Cette page montre comment la voir et la dessiner sous le [piano roll](/docs/making-music/piano-roll), et comment utiliser les contrôleurs MPE, qui envoient de l’expression pour chaque note séparément.

## choisir un couloir {#choosing-a-lane}

La zone sous la grille de notes affiche un couloir à la fois. Choisissez-le avec le sélecteur de couloir à sa gauche :

- **notes** : **vélocité** (par défaut ; voir [la vélocité](/docs/making-music/piano-roll#the-velocity-lane)), et les couloirs par note **pression de note**, **hauteur de note** et **timbre de note**.
- **expression du clip** : **pitch bend**, **pression de canal**, **cc 1** (molette de modulation), **cc 11** (expression) et **cc 64** (sustain), tout autre contrôleur que le clip utilise déjà, et **autre cc…** pour choisir n’importe quel numéro de contrôleur de 0 à 119.
- **enveloppes de ce clip** : les enveloppes d’appareil du clip, s’il en a. Voir [automation](/docs/making-music/automation).

Un **•** après le nom d’un couloir signifie qu’il contient déjà des données dans ce clip.

Les couloirs d’expression du clip appartiennent au clip : ils se déplacent et bouclent avec lui, et leur timing s’aligne sur les notes.

## dessiner et modifier des courbes {#drawing-and-editing-curves}

Dans un couloir d’expression du clip :

- **Dessiner** : faites glisser sur le fond pour dessiner une courbe au crayon.
- **Déplacer un point** : faites-le glisser. Il est aimanté à la grille ([[key:Alt]] : sans aimantation, et il se déplace dix fois moins que la souris, pour des valeurs fines).
- **Ajouter ou supprimer un point** : double-cliquez sur le fond pour en ajouter un, ou double-cliquez sur un point pour le supprimer.
- **Vider le couloir** : faites un clic droit dessus et choisissez **effacer**. Ethereal demande confirmation d’abord.
- **Retirer le couloir** : cliquez sur **retirer le couloir**, ou faites un clic droit et choisissez **retirer le couloir**. Un couloir qui contient des points demande confirmation ; un couloir vide disparaît aussitôt.

Chaque trait ou glissement est une étape de l’[historique d’annulation](/docs/basics/undo-and-history).

Si vous avez vidé ou retiré un couloir par erreur, annulez avec [[action:edit.undo]].

Le pitch bend, la molette de modulation et les autres contrôleurs que vous enregistrez ou [capturez](/docs/making-music/capture-midi) apparaissent aussi ici.

## couloirs par note {#per-note-lanes}

**Pression de note**, **hauteur de note** et **timbre de note** contiennent une courbe pour chaque note, et non une seule pour tout le clip. Dans ces couloirs :

- Un trait de crayon n’écrit que sur les **notes sélectionnées**. Quand aucune note n’est sélectionnée, il écrit sur toutes les notes affichées. Sélectionnez d’abord les notes voulues.
- Faites un clic droit et choisissez **effacer** pour effacer les courbes des notes sélectionnées, ou de toutes les notes affichées si aucune n’est sélectionnée. Ethereal demande confirmation d’abord.
- Les points de courbe qui dépassent la fin d’une note sont conservés mais pas lus.

## mpe {#mpe}

Les contrôleurs MPE (MIDI Polyphonic Expression), comme un Seaboard ou un Linnstrument, envoient la hauteur, la pression et le timbre de chaque note que vous jouez. Pour les enregistrer, activez le MPE sur la piste :

1. Sélectionnez la piste MIDI.
2. Dans l’inspecteur, trouvez la section **mpe** et cochez **activé**.
3. Vérifiez les réglages par rapport à votre contrôleur :
   - **zone** : **basse (master 1)** ou **haute (master 16)**,
   - **canaux** : le nombre de canaux membres (jusqu’à 15),
   - **bend de note** : l’amplitude du pitch bend par note (48 demi-tons par défaut),
   - **bend master** : l’amplitude du pitch bend du canal master.

Avec le MPE activé, la hauteur, la pression et le timbre par note que vous enregistrez deviennent des courbes par note. Les courbes de hauteur sont aussi dessinées à l’intérieur des notes dans le piano roll, sous la forme d’une ligne qui se déplace d’une rangée par demi-ton. Ces lignes sont en lecture seule : modifiez les courbes dans le couloir **hauteur de note** en dessous.

> [!note]
> Si vous enregistrez depuis un contrôleur MPE avant d’activer le MPE, l’expression par note n’est pas enregistrée sous forme de courbes par note. Activez-le d’abord.

## et ensuite {#where-next}

- [Automation](/docs/making-music/automation) : faire évoluer les paramètres des appareils dans le temps.
- [Enregistrement](/docs/making-music/recording) : enregistrer une interprétation avec son expression.
- [Instruments](/docs/making-music/instruments) : quels instruments réagissent à l’expression.
