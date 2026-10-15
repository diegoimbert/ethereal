---
title: arrangement : sélections et modifications temporelles
order: 100
summary: Utilisez la grille, sélectionnez des clips ou une plage de temps, coupez, copiez, dupliquez et insérez des sections entières du morceau, et marquez ses parties.
platform: all
audience: everyone
covers:
  - arrangement/grid-and-snap-alt-bypasses-everywhere #the-grid-and-snapping
  - arrangement/plain-drag-selects-clips-shift-drag-selects-time #selecting-clips-or-time
  - arrangement/plain-edit-keys-act-on-the-time-selection-when-one-exists #edit-keys-and-the-time-selection
  - arrangement/time-edits-insert-silence-delete-time-duplicate-paste #time-edits
  - arrangement/selecting-all-tracks-moves-markers-and-tempo-too #all-tracks-or-some
  - arrangement/paste-overwrites-time-clipboard-is-not-saved #pasting-time
  - arrangement/undo-restores-selections #undo-brings-your-selection-back
  - arrangement/markers-locators #markers
  - arrangement/arrangement-shortcuts-need-the-arrangement-focused #shortcuts-need-the-arrangement
---

Dès qu’un morceau compte quelques parties, vous voudrez déplacer des sections entières : allonger le refrain, retirer une mesure partout ou répéter l’intro. Cette page couvre la grille, les deux types de sélection, les modifications temporelles et les marqueurs. Lisez d’abord [clips](/docs/making-music/clips).

## la grille et le magnétisme {#the-grid-and-snapping}

Le sélecteur **grille**, dans le coin au-dessus des en-têtes de piste, règle la grille de l’arrangement :

- **adaptative (fine)**, **adaptative** et **adaptative (large)** suivent le zoom : plus vous zoomez, plus la grille est fine.
- **1 mesure**, **1/4**, **1/8** et **1/16** sont des pas fixes.
- **désactivé** désactive le magnétisme.

Tout glissement dans l’arrangement se cale sur la grille : déplacer et redimensionner des clips, l’édition du contenu par glissement, l’accolade de boucle, les marqueurs, les points de tempo, les sélections temporelles et les balayages de prises. Maintenez [[key:Alt]] en faisant glisser ou en cliquant pour ignorer la grille le temps de ce geste.

## sélectionner des clips ou du temps {#selecting-clips-or-time}

L’arrangement a deux types de sélection :

- **Clips** : faites glisser sur l’espace vide d’un couloir pour tracer un cadre. Les clips qu’il touche sont sélectionnés.
- **Temps** : maintenez [[key:Shift]] et faites glisser. Cela sélectionne une plage de temps sur les pistes que vous survolez, quels que soient les clips qui s’y trouvent. La sélection traverse les clips.

Vous pouvez appuyer sur [[key:Shift]] ou le relâcher pendant le glissement pour passer de l’une à l’autre. Un clic efface la sélection temporelle et place plutôt le marqueur d’insertion (voir [lire et se déplacer](/docs/basics/playing-and-navigating)).

## les raccourcis d’édition et la sélection temporelle {#edit-keys-and-the-time-selection}

Quand une sélection temporelle existe, les raccourcis d’édition simples agissent sur cette section plutôt que sur des clips entiers : [[action:edit.copy]], [[action:edit.cut]], [[action:edit.paste]], [[action:edit.duplicate]] et [[action:edit.delete]]. Les clips sont coupés aux bords de la sélection, et seule cette plage est copiée ou supprimée.

Sans sélection temporelle, les mêmes raccourcis agissent sur les clips (ou les pistes) sélectionnés. [[action:edit.paste]] colle ce que vous avez copié en dernier : des clips, des pistes ou du temps.

## modifications temporelles {#time-edits}

Elles agissent sur la sélection temporelle. Faites un clic droit à l’intérieur pour obtenir la même liste sous forme de menu.

| modification | raccourci | ce qu’elle fait |
|---|---|---|
| couper la durée | [[action:edit.cut]] ou [[action:time.cut]] | Copie la section, puis la supprime. La suite se décale vers la gauche. |
| copier la durée | [[action:edit.copy]] ou [[action:time.copy]] | Copie la section. |
| coller après la sélection | [[action:edit.paste]] | Colle la section copiée juste après la sélection, par-dessus ce qui s’y trouve. |
| coller la durée | [[action:time.paste]] | Insère la section copiée au début de la sélection et pousse la suite vers la droite. |
| scinder à la sélection | [[action:edit.split]] | Scinde les clips aux deux bords de la sélection. |
| dupliquer la durée | [[action:edit.duplicate]] ou [[action:time.duplicate]] | Place une copie de la section juste après elle, par-dessus ce qui s’y trouve. |
| insérer un silence | [[action:time.insertSilence]] | Insère au début de la sélection un temps vide aussi long qu’elle. Tout ce qui suit se décale vers la droite. |
| supprimer la durée | [[action:edit.delete]] ou [[action:time.delete]] | Supprime la section. La suite se décale vers la gauche. |

Le menu propose aussi **coller par-dessus la sélection** et les commandes de rendu (voir [geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce)). Chaque modification temporelle est une seule étape d’annulation.

### toutes les pistes ou certaines {#all-tracks-or-some}

Si la sélection couvre toutes les pistes, une modification temporelle change tout le morceau : les marqueurs, les changements de tempo et les changements de signature rythmique se déplacent avec elle. Si elle ne couvre que certaines pistes, seuls leurs clips, leurs prises et leur automation se déplacent.

### coller du temps {#pasting-time}

Coller et dupliquer écrivent par-dessus ce qui est déjà là. Pour pousser plutôt le contenu existant vers la droite, utilisez **coller la durée** ([[action:time.paste]]).

Le presse-papiers temporel vit dans le moteur, pour cette session seulement. Il est perdu quand vous redémarrez l’application.

## l’annulation rétablit votre sélection {#undo-brings-your-selection-back}

[[action:edit.undo]] et [[action:edit.redo]] rétablissent aussi la sélection que vous aviez à cette étape : la sélection temporelle, la section du piano roll, ainsi que les clips et les notes sélectionnés. Vous pouvez donc annuler une modification et la refaire aussitôt sur la même plage.

## marqueurs {#markers}

Les marqueurs nomment les parties d’un morceau (intro, couplet, refrain) et vous permettent d’y sauter. Ils se trouvent dans le couloir **marqueurs**, au-dessus des pistes.

- Double-cliquez sur un espace vide du couloir pour y ajouter un marqueur, ou cliquez sur **+** dans le coin du couloir. **+** l’ajoute au marqueur d’insertion, ou à la tête de lecture pendant la lecture, pour que vous puissiez marquer ce que vous entendez.
- Cliquez sur un marqueur pour y sauter.
- Faites glisser un marqueur pour le déplacer. Il se cale sur la grille ([[key:Alt]] : librement).
- Double-cliquez sur un marqueur pour le renommer.
- Faites un clic droit sur un marqueur pour **aller au marqueur**, **renommer** et **supprimer le marqueur**.

## les raccourcis ont besoin de l’arrangement {#shortcuts-need-the-arrangement}

Les raccourcis des clips et du temps (scinder, dupliquer, supprimer, les flèches pour les prises) fonctionnent quand l’arrangement a le focus clavier. Cliquer n’importe où dedans lui donne le focus. [[action:edit.deselect]] efface la sélection et ferme le piano roll depuis n’importe où.

## et ensuite {#where-next}

- [automation](/docs/making-music/automation) : faites varier les réglages dans le temps.
- [les raccourcis clavier](/docs/reference/keyboard-shortcuts) : tous les raccourcis au même endroit.
- [annulation et historique](/docs/basics/undo-and-history) : revenez à n’importe quelle étape.
