---
title: gestes tactiles
order: 20
summary: Faites défiler, zoomez, sélectionnez, déplacez, ajustez et décalez les clips avec vos doigts dans la bêta mobile.
platform: mobile
audience: everyone
covers:
  - mobile/pan-pinch-and-momentum #scrolling-and-zooming
  - mobile/tap-double-tap-long-press #tap-double-tap-and-long-press
  - mobile/panning-stops-following-the-playhead #following-the-playhead
  - mobile/empty-arrangement-double-tap-to-make-a-clip #making-a-clip
  - mobile/track-headers-on-touch #track-headers
  - mobile/the-track-sheet #the-track-sheet
  - mobile/sheets-close-with-back #sheets
  - mobile/moving-clips-select-first-or-long-press #moving-clips
  - mobile/trim-handles-only-on-selected-wide-enough-clips #trimming-clips
  - mobile/the-clip-action-bar #the-clip-action-bar
  - mobile/select-more-multi-select-without-shift #selecting-several-clips
  - mobile/nudge-by-one-grid-step #nudging-by-one-grid-step
  - mobile/time-range-with-handles #selecting-a-time-range
  - mobile/clip-sheet-typed-position #typing-a-position
---

Un téléphone n’a ni souris, ni clic droit, ni touche Maj, alors la [bêta mobile](/docs/mobile/mobile-beta) les remplace par des gestes. Cette page couvre l’arrangement : défilement et zoom, pistes et clips. La modification des notes, de l’automation et du mixeur est décrite dans [modifier au toucher](/docs/mobile/editing-on-touch).

## toucher, toucher deux fois et appui long {#tap-double-tap-and-long-press}

Trois gestes remplacent la souris :

| geste | ce qu’il fait | sur un ordinateur |
|---|---|---|
| toucher | sélectionne, appuie sur un bouton | clic |
| toucher deux fois (deux touchers en 0,3 s environ) | ouvre ou crée | double-clic |
| appui long (rester immobile environ une demi-seconde) | ouvre le menu de l’élément | clic droit |

Sur Android, un appui long produit une courte vibration quand il est reconnu. Si votre doigt bouge de plus de quelques pixels, le toucher devient un glissement.

## défilement et zoom {#scrolling-and-zooming}

- **Glissez avec un doigt** pour faire défiler. Horizontalement, vous avancez dans le temps ; verticalement, vous parcourez les pistes. Un glissement qui part surtout dans une direction y reste verrouillé, donc un balayage horizontal ne dérive pas vers le haut ou le bas.
- **Donnez une impulsion** et la vue continue de glisser après que vous avez levé le doigt. Touchez l’écran pour l’arrêter.
- **Pincez avec les doigts côte à côte** pour zoomer et dézoomer dans le temps, autour du point situé entre vos doigts.
- **Pincez avec un doigt au-dessus de l’autre** pour agrandir ou réduire la hauteur des pistes.
- **Glissez sur la règle** en haut pour déplacer la tête de lecture (scrubbing).

## suivre la tête de lecture {#following-the-playhead}

Dans la bêta mobile, la vue suit la tête de lecture par défaut, pour que la musique reste à l’écran pendant la lecture. Dès que vous faites défiler ou pincez pendant la lecture, le suivi s’arrête, pour que vous puissiez regarder ailleurs. Touchez le bouton **suivre la tête de lecture** (dans la ligne d’état au-dessus du transport) pour le suivre de nouveau.

## créer un clip {#making-a-clip}

L’arrangement d’un nouveau projet est vide et indique **touchez deux fois le couloir d’une piste MIDI pour créer un clip**. Touchez deux fois le couloir d’une piste MIDI pour y créer un clip MIDI vide, puis touchez deux fois le clip pour [modifier ses notes](/docs/mobile/editing-on-touch#the-piano-roll-on-touch). Il n’y a pas de navigateur d’échantillons sur mobile, donc les clips audio viennent de projets créés sur un ordinateur.

## en-têtes de piste {#track-headers}

- **Touchez** un en-tête pour sélectionner la piste.
- **Touchez de nouveau l’en-tête sélectionné** pour ouvrir sa [fiche de piste](#the-track-sheet).
- **Appui long** sur un en-tête pour le menu de la piste.
- **Appui long, puis glissez** un en-tête pour déplacer la piste vers le haut ou le bas, ou dans un groupe.

Sur un téléphone, un en-tête affiche le nom de la piste et ses boutons **muet**, **solo** et **armer**, toujours visibles et en taille tactile. Touchez-les directement. Une tablette affiche aussi le fader de volume et le bouton de panoramique.

## la fiche de piste {#the-track-sheet}

La fiche de piste contient tout ce qu’offre un en-tête de piste sur ordinateur, en taille tactile :

- **muet**, **solo** et **armer**, et **automation**, qui affiche les [couloirs d’automation](/docs/mobile/editing-on-touch#automation-on-touch) de la piste.
- Un fader de **volume** et un bouton de **panoramique** plus grands.
- **nom** : touchez pour renommer la piste.
- **monter** et **descendre**, et les actions du menu de la piste : couper, copier, coller, dupliquer et supprimer.
- **appareils** : les appareils de la piste, avec activé ou désactivé. Vous ne pouvez pas encore les modifier sur mobile (« pour l’instant, modifiez les appareils sur un ordinateur »).

## fiches {#sheets}

Les fiches (la fiche de piste, la position d’un clip) glissent depuis le bas de l’écran :

- Faites glisser la poignée du haut vers le haut pour agrandir la fiche, ou vers le bas pour la réduire ou la fermer.
- Touchez en dehors de la fiche pour la fermer.
- Le bouton ou le geste retour du système ferme la fiche au lieu de quitter le projet. Il ferme aussi l’éditeur de clip.

En mode paysage, une fiche occupe tout l’écran.

## déplacer des clips {#moving-clips}

Un simple glissement fait défiler la vue, même sur un clip, pour que vous puissiez parcourir un arrangement chargé sans rien déplacer par accident. Pour déplacer un clip :

- **Touchez-le d’abord** pour le sélectionner, puis faites-le glisser. Faire glisser un clip sélectionné déplace toute la sélection.
- Ou **faites un appui long, puis glissez** sans lever le doigt. Cela fonctionne sur un clip qui n’est pas sélectionné.

Pendant le glissement, un badge au-dessus de votre doigt montre où le clip va atterrir (aligné sur la grille). Glissez près d’un bord de l’écran pour faire défiler dans cette direction. Glissez sous la dernière piste pour placer le clip sur une nouvelle piste. Pour annuler un glissement, posez un deuxième doigt.

## ajuster des clips {#trimming-clips}

Seul un clip sélectionné a des poignées d’ajustement : une large zone tactile sur chaque bord, à moitié dans le clip et à moitié dehors. Faites-en glisser une pour raccourcir ou allonger le clip de ce côté.

Un clip très étroit à l’écran (moins de 24 pixels environ) n’a pas de poignées. Zoomez, ou saisissez son début et sa durée dans la [fiche du clip](#typing-a-position).

## la barre d’actions des clips {#the-clip-action-bar}

Quand des clips sont sélectionnés, une barre apparaît en bas de l’arrangement avec ce que vous feriez au clavier ou par un clic droit sur un ordinateur :

- **sélectionner plus** : sélectionner plusieurs clips (voir plus bas).
- **modifier** : ouvrir le clip dans l’éditeur.
- **scinder**, **dupliquer**, **copier**, **coller** et **supprimer**.
- **plus tôt** et **plus tard** : [décaler](#nudging-by-one-grid-step) d’un pas de grille.
- **position** : saisir le début et la durée du clip (un seul clip).
- **plus** : le menu complet du clip, tel qu’un clic droit l’affiche sur un ordinateur.

## sélectionner plusieurs clips {#selecting-several-clips}

Il n’y a ni Maj ni Ctrl sur un téléphone, alors touchez **sélectionner plus** dans la barre des clips. Jusqu’à ce que vous ayez terminé :

- Toucher un clip l’ajoute à la sélection ou l’en retire.
- Un glissement sur un espace vide du couloir trace un rectangle de sélection.

Le bouton affiche alors **terminé** et le nombre de clips sélectionnés : touchez-le pour revenir aux touchers normaux. Le mode sélectionner plus se termine aussi de lui-même quand plus aucun clip n’est sélectionné.

## décaler d’un pas de grille {#nudging-by-one-grid-step}

**plus tôt** et **plus tard** déplacent les clips sélectionnés d’un pas de grille au zoom actuel (un temps quand la grille est désactivée). Utilisez-les quand un glissement est trop grossier pour votre doigt. Les clips ne passent jamais avant le début du morceau.

## sélectionner une plage de temps {#selecting-a-time-range}

Sur un ordinateur, vous glissez avec Maj pour sélectionner une portion de temps. Sur un téléphone, **faites un appui long sur un espace vide du couloir, puis glissez**. La plage reçoit deux poignées, comme pour sélectionner du texte : faites glisser l’une ou l’autre extrémité pour la modifier (elle s’aligne sur la grille). Faire passer une poignée au-delà de l’autre les inverse.

Une plage a sa propre barre : **copier**, **couper**, **coller**, **dupliquer**, **boucler** (règle la boucle sur la plage et active la lecture en boucle), **supprimer le temps** et **plus**. Ce que font ces actions est expliqué dans [arrangement](/docs/making-music/arranging).

## saisir une position {#typing-a-position}

**position** dans la barre des clips ouvre une fiche où vous saisissez des nombres au lieu de faire glisser :

- **début** : l’endroit où commence le clip, en mesure.temps.double-croche, compté à partir de 1. `3` est le début de la mesure 3, `3.2` en est le deuxième temps.
- **durée** : sa longueur, en mesures.temps.double-croches, comptée à partir de 0. `1` vaut une mesure, `0.2` vaut deux temps.
- **bord de début** et **bord de fin** : **plus tôt** et **plus tard** déplacent ce bord d’un pas de grille.

Appuyez sur Entrée ou touchez ailleurs pour appliquer une valeur saisie. Une valeur qui n’a pas de sens est remise comme avant. Chaque changement est une étape que vous pouvez annuler.

## et ensuite {#where-next}

- [Modifier au toucher](/docs/mobile/editing-on-touch) : les notes, l’automation et le mixeur.
- [Clips](/docs/making-music/clips) : ce que sont les clips et comment ils se comportent.
- [Annuler et historique](/docs/basics/undo-and-history).
