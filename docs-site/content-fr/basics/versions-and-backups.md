---
title: versions et sauvegardes
order: 50
summary: Revenez à un état antérieur de votre projet grâce aux versions automatiques et aux instantanés nommés, et récupérez votre travail après un plantage.
platform: all
audience: beginner
covers:
  - projects-storage/versions-automatic-backups #automatic-versions
  - projects-storage/restoring-a-version-is-itself-reversible #restoring-a-version
  - projects-storage/versions-don-t-include-audio-files #what-a-version-contains
  - projects-storage/save-snapshot-mod-alt-s #snapshots
  - projects-storage/crash-recovery-dialog #after-a-crash
---

En plus de l’[annulation](/docs/basics/undo-and-history), qui ne remonte que les modifications faites depuis l’ouverture du projet, Ethereal garde d’anciennes **versions** de chaque projet. Vous pouvez les comparer avec l’état actuel et en restaurer une. Cette page explique aussi ce qui se passe après un plantage.

## versions automatiques {#automatic-versions}

Pendant que vous travaillez, Ethereal garde une version du projet environ une fois par minute, tant que quelque chose a changé. Vous n’avez rien à faire.

Pour garder la liste courte, les anciennes versions sont espacées : au plus 30 versions automatiques sont conservées, réparties des dernières minutes jusqu’à la toute première. Les versions que vous avez nommées (les instantanés) ne sont jamais supprimées.

Ouvrez la liste depuis le menu du projet (☰) › **versions…**, ou avec **versions…** dans la palette de commandes. Les versions les plus récentes sont en haut. Pour chacune, vous pouvez :

- la **comparer** avec le projet tel qu’il est maintenant : la fenêtre liste ce qui a été ajouté, retiré ou modifié depuis ;
- la **restaurer** ;
- depuis son menu **⋯**, la **renommer** (ce qui la conserve aussi définitivement) ou la **supprimer…**.

Le champ en haut de la fenêtre enregistre une version du projet tel qu’il est maintenant, avec un nom facultatif.

## restaurer une version {#restoring-a-version}

**Restaurer** remplace le projet par cette version. Avant cela, Ethereal garde l’état actuel comme une version marquée **avant restauration**. Une restauration n’est donc jamais sans retour : si vous changez d’avis, restaurez la version **avant restauration**.

Seules les 10 versions **avant restauration** non nommées les plus récentes sont conservées.

## ce que contient une version {#what-a-version-contains}

Une version contient le projet lui-même : les pistes, clips, notes, appareils, l’automation et les réglages. Elle ne contient **pas** vos fichiers audio ni les fichiers exportés.

> [!warning]
> Si vous supprimez ou remplacez un fichier audio, restaurer une version plus ancienne ne le fera pas revenir. Les clips qui l’utilisaient apparaissent comme manquants.

## instantanés {#snapshots}

Un instantané est une version avec un nom, comme « avant le mixage final », conservée jusqu’à ce que vous la supprimiez. Pour en créer un :

- menu du projet › **enregistrer un instantané…** ;
- [[action:project.snapshot]] ;
- **enregistrer un instantané…** dans la palette de commandes.

Tapez un nom et appuyez sur **enregistrer l’instantané**. Il apparaît dans **versions…** avec son nom.

## après un plantage {#after-a-crash}

Si Ethereal ne s’est pas fermé correctement la dernière fois (un plantage, une coupure de courant), le lancement suivant affiche une fenêtre avant toute chose.

Si un projet a des modifications plus récentes que son dernier enregistrement, la fenêtre s’intitule **récupérer le travail non enregistré ?**, avec trois choix pour chaque projet :

- **récupérer** ouvre ce travail plus récent, non enregistré. Il n’est pas encore enregistré : vérifiez que c’est bien ce que vous voulez, puis enregistrez ([[action:project.save]]) pour le garder.
- **garder la version enregistrée** oublie le travail non enregistré et garde le dernier état enregistré.
- **ouvrir sans plug-ins** ouvre le projet enregistré avec ses plug-ins désactivés, au cas où un plug-in aurait causé le plantage (voir [le mode sans échec](/docs/advanced/safe-mode)).

S’il n’y avait pas de travail non enregistré (par exemple, un plug-in a planté pendant le chargement du projet), la fenêtre s’intitule **Ethereal ne s’est pas fermé correctement**, avec **ouvrir**, **ouvrir sans plug-ins** et **masquer**.

**Décider plus tard** ferme la fenêtre sans choisir ; elle vous redemandera au prochain lancement.

## et ensuite {#where-next}

- [Annuler et historique](/docs/basics/undo-and-history)
- [Déplacer des projets entre ordinateurs](/docs/basics/moving-projects)
- [Le mode sans échec](/docs/advanced/safe-mode)
