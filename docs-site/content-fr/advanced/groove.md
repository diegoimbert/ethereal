---
title: groove, swing et humanisation
order: 50
summary: Ajoutez du swing à tout le morceau à la lecture, écrivez le swing dans un clip, et assouplissez le timing et la vélocité.
platform: all
audience: advanced
covers:
  - midi-editing/quantize-and-humanize-settings-are-not-saved #the-settings-are-remembered
  - midi-editing/humanize-randomizes-timing-and-velocity #humanize
  - midi-editing/project-swing-vs-quantize-swing #project-swing-vs-quantize-swing
---

Des notes dessinées sur une grille peuvent sonner raides. Ethereal a trois outils pour donner du feeling au MIDI : le swing de lecture du projet, le swing de **quantifier…**, et **humaniser…**. La quantification elle-même est expliquée dans [le piano roll](/docs/making-music/piano-roll).

## swing du projet ou swing de quantification {#project-swing-vs-quantize-swing}

Il y a deux sortes de swing, et elles fonctionnent différemment :

- Le **swing du projet** retarde les notes à contretemps de tous les clips MIDI pendant la lecture du morceau. Aucune note ne bouge : remettez-le à 0 % et tout redevient droit. Réglez-le dans la ligne **groove** de l’inspecteur quand un clip MIDI est sélectionné : une quantité en pourcentage et une grille (**1/8** ou **1/16**). Il est enregistré avec le projet.
- **Le swing de quantifier…** écrit le swing dans les notes d’un clip : les notes à contretemps bougent vraiment. Réglez **swing** dans la fenêtre **quantifier…** du piano roll, puis quantifiez les notes ([[action:pianoRoll.quantize]]).

Utilisez le swing du projet pour essayer un feeling sur tout le morceau, et le swing de quantification quand une partie a besoin du sien.

## humaniser {#humanize}

**humaniser…** dans la barre d’outils du piano roll (ou **humaniser** dans le menu du clic droit d’une note) décale les notes au hasard dans le temps et fait varier leur vélocité :

- **timing (±1/16)** : à 100 %, une note peut avancer ou reculer jusqu’à une double croche.
- **vélocité** : de combien les vélocités varient.

Cela s’applique aux notes sélectionnées, ou à tout le clip quand rien n’est sélectionné. Chaque passage donne un résultat aléatoire différent, et compte comme une étape d’annulation.

## les réglages sont mémorisés {#the-settings-are-remembered}

Les valeurs que vous réglez dans les fenêtres **quantifier…** et **humaniser…** sont mémorisées sur cet ordinateur, même après un redémarrage, et [[action:pianoRoll.quantize]] les utilise. Elles appartiennent à l’application, pas à un projet : tous les projets utilisent les mêmes. Le swing du projet, lui, est enregistré avec le projet.

## et ensuite {#where-next}

- [le piano roll](/docs/making-music/piano-roll) : dessiner des notes et quantifier.
- [audio vers midi](/docs/advanced/audio-to-midi) : transformez un enregistrement en notes.
