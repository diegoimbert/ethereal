---
title: clips audio : fondus, warp et inversion
order: 90
summary: Façonnez les clips audio avec des fondus et des fondus enchaînés, réglez leur gain, jouez-les à l’envers et faites-les suivre le tempo avec le warp.
platform: all
audience: everyone
covers:
  - clips/fade-handles-on-audio-clips #fades
  - clips/crossfades-between-touching-clips #crossfades
  - clips/clip-gain-changes-the-waveform-height #clip-gain
  - clips/reverse #reverse
  - clips/warp-on-off-repitch-or-complex #warp-on-or-off
  - clips/warp-markers #warp-markers
---

Les clips audio ont quelques modifications qui leur sont propres, en plus de celles communes à tous les clips (voir [clips](/docs/making-music/clips)). Sélectionnez un clip audio pour suivre. La plupart de ces réglages se trouvent aussi dans l’inspecteur.

## fondus {#fades}

Survolez un clip audio, ou sélectionnez-le, pour voir ses poignées de fondu aux coins supérieurs du corps du clip.

- Faites glisser une poignée latéralement pour régler la durée du fondu d’entrée ou de sortie.
- Faites glisser la petite poignée au milieu d’un fondu vers le haut ou le bas pour courber sa forme. Double-cliquez dessus pour rendre le fondu de nouveau droit (linéaire).
- Faites un clic droit sur le clip et choisissez **fondus à puissance constante** ou **fondus linéaires** pour changer les courbes de tous les clips sélectionnés.

Les champs **fondu d’entrée** et **fondu de sortie** de l’inspecteur acceptent des durées exactes.

## fondus enchaînés {#crossfades}

Un fondu enchaîné mêle la fin d’un clip au début du suivant sur la même piste.

- Faites glisser une poignée de fondu au-delà du bord du clip, dans un clip qui le touche ou le chevauche. Cela crée un fondu enchaîné, centré sur la limite.
- Faites glisser la poignée d’un fondu enchaîné existant pour le redimensionner. Ramenez-la presque à zéro pour le supprimer.
- Ou faites un clic droit sur un clip : **fondu enchaîné avec le clip précédent** et **fondu enchaîné avec le clip suivant** créent un fondu enchaîné d’un demi-temps (plus court pour les clips courts). **supprimer le fondu enchaîné avec le clip précédent** / **avec le clip suivant** le retirent.

## gain du clip {#clip-gain}

Le champ **gain** de l’inspecteur monte ou baisse le niveau d’un clip. La forme d’onde grandit ou rétrécit avec lui, si bien que ce que vous voyez correspond à ce que vous entendez. Les parties qui dépasseraient la pleine échelle sont dessinées aplaties, avec une marque de la couleur du clip, pour avertir qu’elles risquent de saturer.

## inversion {#reverse}

Appuyez sur [[action:clip.reverse]] (ou faites un clic droit et choisissez **inverser**) pour jouer les clips sélectionnés à l’envers.

- Un clip audio affiche un badge **rev** dans sa barre de titre. Appuyez de nouveau (**rétablir le sens**) pour le jouer à l’endroit. Un clip raccourci joue la même partie du fichier, à l’envers. Les fondus restent aux bords du clip.
- Un clip MIDI voit ses notes et ses contrôleurs inversés dans le temps. Cela modifie les notes : appuyez sur [[action:edit.undo]] pour les retrouver.

L’inspecteur a aussi un interrupteur **inverser**.

## warp {#warp}

Le warp fait suivre à un clip audio le tempo du projet : changez le tempo et le clip accélère ou ralentit avec lui. Cliquez sur un clip audio pour ouvrir son éditeur en bas de la fenêtre, sur l’onglet **warp**.

### warp activé ou désactivé {#warp-on-or-off}

- La case **warp** active ou désactive le warp. Sans warp, le clip joue à sa vitesse d’origine, quel que soit le tempo.
- **mode** choisit la façon dont l’audio est étiré :
  - **repitch** change la vitesse et la hauteur ensemble, comme un disque joué plus vite ou plus lentement.
  - **complex** garde la hauteur en changeant la vitesse. Utilisez-le pour les voix et les mixages complets.
- La barre d’outils affiche le tempo qu’Ethereal a détecté dans le fichier (**source 120.00 bpm**), ou **tempo source inconnu**.
- **transposer** change la hauteur du clip en demi-tons.

Si le moteur que vous utilisez ne sait pas étirer le temps, la barre d’outils indique **complex est joué en repitch ici**.

### marqueurs de warp {#warp-markers}

Les marqueurs de warp fixent un instant de l’audio sur un temps, pour corriger un timing qui dérive. Avec le warp activé :

- Double-cliquez sur la forme d’onde pour ajouter un marqueur.
- Faites glisser un marqueur pour amener cet instant de l’audio sur un autre temps. Il se cale sur les doubles croches. Maintenez [[key:Alt]] pour le déplacer librement.
- Double-cliquez sur un marqueur, ou sélectionnez-le et appuyez sur [[key:Delete]], pour le supprimer.

Un marqueur n’étire l’audio qu’entre ses voisins. Le premier marqueur que vous ajoutez fixe aussi le début et la fin du fichier, pour que le reste du clip ne bouge pas.

## et ensuite {#where-next}

- [clips](/docs/making-music/clips) : déplacer, raccourcir, scinder et boucler.
- [automation](/docs/making-music/automation) : faites varier le volume et d’autres réglages dans le temps.
- [prises et comping](/docs/making-music/takes-and-comping) : construisez une seule interprétation à partir de plusieurs enregistrements.
