---
title: thèmes
order: 85
summary: Choisissez l’un des six thèmes intégrés, suivez le mode clair ou sombre de votre système, ou créez, partagez et importez les vôtres.
platform: all
audience: everyone
---

Un thème fixe toutes les couleurs de l’interface : les panneaux, le texte, la couleur d’accent, l’arrangement, les vumètres et les formes d’onde. Choisissez-le dans les réglages, sous **apparence** › **thème**. Il s’applique aussitôt et il est gardé sur cet appareil.

## thèmes intégrés {#built-in-themes}

Cliquez sur une carte pour utiliser son thème :

| thème | son aspect |
|---|---|
| **sombre** | un anthracite doux avec un accent orchidée (par défaut) |
| **obsidienne** | presque noir avec un accent cyan, pour les pièces sombres et les écrans OLED |
| **braise** | un anthracite brun et chaud avec un accent ambré |
| **marée** | un bleu marine profond avec un accent pervenche |
| **crépuscule** | des surfaces ardoise plus claires et un texte plus doux, avec un accent rose |
| **clair** | un gris clair avec un accent orchidée |

Chaque thème intégré garde le texte et les commandes lisibles : tous atteignent les niveaux de contraste WCAG AA.

Les commandes **passer au thème clair** et **passer au thème sombre** de la [palette de commandes](/docs/basics/command-palette-and-settings) passent au dernier thème clair ou sombre que vous avez utilisé.

## suivre le système {#following-the-system}

Activez **suivre le mode clair ou sombre du système** pour changer de thème en même temps que votre système (au coucher du soleil, par exemple). Choisissez le thème de chaque mode sous **thème clair** et **thème sombre**. Cliquer sur une carte de thème arrête de suivre le système.

## créer votre thème {#make-your-own-theme}

Cliquez sur **créer votre thème** : il commence comme une copie du thème que vous utilisez. Dans l’éditeur :

- **nom** : ce qu’affiche sa carte.
- **base** : sombre ou clair. Une base claire désactive aussi le halo et utilise des ombres plus claires.
- **couleurs principales** : l’arrière-plan, les panneaux, les surfaces en relief, le texte, le texte atténué, l’accent, et les couleurs de succès, d’avertissement, de danger et d’information. Cliquez sur une pastille pour choisir une couleur, ou tapez-la (`#1c1f26`, `rgba(…)`).
- **tous les jetons** : toutes les autres couleurs de l’application. Chacune est calculée à partir des couleurs principales (teintes de survol, bordures, tons plus clairs et plus sombres de l’accent, vumètres…) tant que vous ne la fixez pas ici ; le bouton de réinitialisation la rend.
- **contraste** : ce qui, dans votre thème, n’atteint pas les niveaux WCAG AA, comme un texte atténué trop pâle sur les panneaux. Corrigez-le pour que tout reste lisible.

Toute l’application montre vos changements pendant que vous modifiez, arrangement et vumètres compris. **enregistrer** garde le thème et l’utilise ; **annuler** revient au thème que vous aviez.

Vos thèmes ont une carte à côté des thèmes intégrés. Son bouton **…** (ou un clic droit) permet de le **modifier**, le **dupliquer**, l’**exporter…** ou le **supprimer**. Les thèmes intégrés peuvent aussi être dupliqués et exportés.

> [!note]
> Vos thèmes sont gardés sur cet appareil : dans ce navigateur sur le web, dans l’application sur ordinateur. Exportez-les pour en garder une copie ou les utiliser ailleurs.

## importer et exporter {#import-and-export}

**exporter…** enregistre un thème dans un fichier `.json` : le navigateur le télécharge, l’application de bureau demande où l’enregistrer. Partagez-le comme n’importe quel fichier. **importer un thème…** le relit : le thème s’ajoute à vos thèmes et s’applique aussitôt.

Un fichier de thème indique sa base et quelques couleurs. Ethereal calcule les couleurs qu’il ne donne pas, et ignore (en vous le disant) les couleurs qu’il ne connaît pas ou les valeurs qui ne sont pas des couleurs. Un fichier créé par une version plus récente d’Ethereal ne peut pas être importé : mettez d’abord Ethereal à jour.

## et ensuite {#where-next}

- [référence des réglages](/docs/advanced/settings#appearance)
- [le format des fichiers de thème](https://github.com/diegoimbert/ethereal/blob/main/docs/THEMES.md#theme-files)
- [la palette de commandes et les réglages](/docs/basics/command-palette-and-settings)
