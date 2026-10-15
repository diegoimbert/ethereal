---
title: mode sans échec
order: 80
summary: Ouvrez un projet avec ses plug-ins désactivés, quand un plug-in le fait planter ou le bloque.
platform: desktop
audience: advanced
covers:
  - first-run/safe-mode-open-without-plugins #opening-without-plugins
---

Un plug-in défectueux peut faire planter Ethereal ou le figer dès qu’un projet s’ouvre. Le mode sans échec ouvre le projet avec tous les plug-ins désactivés : vous pouvez y entrer, trouver le coupable et sauver votre travail.

## ouvrir sans plug-ins {#opening-without-plugins}

Il y a deux façons d’y accéder :

- Sur l’écran d’accueil, faites un clic droit sur un projet récent et choisissez **ouvrir sans plug-ins**.
- Après un plantage, la fenêtre de récupération liste les projets qui étaient ouverts et propose **ouvrir sans plug-ins** pour chacun.

Le projet s’ouvre avec tous les plug-ins contournés : ils ne produisent aucun son et ne tournent pas, mais leurs réglages sont conservés. Les appareils intégrés fonctionnent normalement.

## en mode sans échec {#while-in-safe-mode}

- La barre du haut indique **plug-ins désactivés (mode sans échec)**.
- Chaque plug-in affiche **mode sans échec · contourné**, avec un bouton **⟳** qui charge ce seul plug-in. Les charger un par un permet de trouver celui qui plante.
- **charger les plug-ins** dans la barre du haut les charge tous et quitte le mode sans échec.

Ouvrir en mode sans échec ne modifie pas le projet enregistré : chaque plug-in garde son état.

## et ensuite {#where-next}

- [les plug-ins](/docs/sounds-and-plugins/plugins) : sandbox, plantages et plug-ins manquants.
- [versions et sauvegardes](/docs/basics/versions-and-backups) : la récupération après un plantage.
- [dépannage](/docs/reference/troubleshooting)
