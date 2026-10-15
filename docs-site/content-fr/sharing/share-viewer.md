---
title: la visionneuse de partage
order: 60
summary: Ce que voient les gens quand ils ouvrent un lien statique, comment ils écoutent des pistes seules, et comment ils ouvrent leur propre copie.
platform: web, mobile
audience: beginner
covers:
  - sharing/the-read-only-viewer #a-read-only-ethereal
  - sharing/solo-and-mute-in-the-viewer #listening-to-single-tracks
  - sharing/open-a-copy-to-edit #opening-a-copy-to-edit
---

Cette page s’adresse à quiconque ouvre un lien de partage statique (une adresse `ethereal.ws/s/…`), et au propriétaire qui veut savoir ce que reçoivent les auditeurs. Pour en publier un, voir [liens de partage statiques](/docs/sharing/static-links).

## un ethereal en lecture seule {#a-read-only-ethereal}

Le lien ouvre le projet dans l’éditeur d’Ethereal lui-même, dans votre navigateur, en lecture seule. Un bandeau en haut affiche le nom du morceau et « partagé avec Ethereal · lecture seule ». Rien à installer, et aucun compte nécessaire.

Vous pouvez lancer la lecture, l’arrêter, boucler et cliquer sur la règle pour déplacer la tête de lecture, et parcourir le projet : l’arrangement, les clips dans le piano roll et les appareils. Rien de ce que vous faites ne modifie le morceau, et les touches qui modifieraient sont ignorées. Ce que vous entendez est l’audio rendu de l’instantané : le MIDI n’est pas joué en direct.

Cela fonctionne aussi sur téléphone, avec des en-têtes de piste et un transport simplifiés.

## écouter des pistes seules {#listening-to-single-tracks}

Si le propriétaire l’a autorisé, les boutons solo et muet de chaque piste fonctionnent : ils jouent l’audio propre à cette piste. Comme dans l’application, un solo est exclusif ; [[key:Mod]]-clic ajoute d’autres solos. **revenir au mix complet** revient au mix.

Le premier solo ou muet télécharge toutes les pistes (« chargement des pistes… le mix complet joue en attendant qu’elles soient prêtes »). Les vumètres des pistes bougent une fois les pistes chargées.

Quand le bandeau indique « le propriétaire n’a partagé que le mix. », seul le mix a été publié, et solo et muet ne font rien.

Le bandeau peut aussi indiquer « le propriétaire a masqué les appareils. » : les chaînes d’appareils ont été retirées de cet instantané.

## télécharger {#downloading}

Si le propriétaire a autorisé les téléchargements, un menu **télécharger** propose le mix (sans perte), le fichier du projet et, quand les pistes ont été partagées, chaque piste.

## ouvrir une copie à modifier {#opening-a-copy-to-edit}

Si le propriétaire a autorisé la modification, le bandeau propose **ouvrir une copie à modifier**. Cela lance l’application web Ethereal et importe le projet comme votre propre projet privé, stocké dans ce navigateur. Vos modifications ne reviennent jamais au propriétaire, et ses modifications ultérieures n’atteignent jamais votre copie.

> [!note]
> Votre copie ne vit que dans ce navigateur, comme tout projet web : voir [où vivent vos projets](/docs/welcome/where-your-projects-live). Sur un téléphone, ouvrir une copie demande la bêta mobile.

## et ensuite {#where-next}

- [liens de partage statiques](/docs/sharing/static-links)
- [qu’est-ce qu’ethereal](/docs/welcome/what-is-ethereal)
