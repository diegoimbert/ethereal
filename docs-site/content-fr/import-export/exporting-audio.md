---
title: exporter l’audio
order: 30
summary: Faites le rendu de votre morceau, d’une boucle ou d’une sélection en WAV, FLAC ou MP3, en un seul mix ou en stems.
platform: all
audience: beginner
covers:
  - export/export-audio-dialog #the-export-audio-dialog
  - export/where-exported-files-go #where-the-files-go
  - export/range-options-and-the-selected-clips-range #range
  - export/tail-for-reverb-and-delay #tail
  - export/formats-bit-depth-and-sample-rate-rules #format-bit-depth-and-sample-rate
  - export/normalize #normalize
  - export/stems-what-each-file-contains #stems
  - export/overlapping-stems #when-stems-overlap
  - export/export-keeps-running-when-the-dialog-is-closed #while-it-exports
  - export/large-exports-in-the-browser #large-exports-in-the-browser
  - export/a-plugin-that-can-t-render-offline-fails-the-export #when-a-plugin-stops-the-export
---

Quand votre morceau est prêt à être partagé, exportez-le en fichier audio. Cette page passe en revue chaque option de la fenêtre d’export, y compris les stems (un fichier par piste). Pour envoyer le projet lui-même vers une autre DAW, voir [exporter vers d’autres DAW](/docs/import-export/exporting-to-other-daws).

## la fenêtre exporter l’audio {#the-export-audio-dialog}

Ouvrez le menu du projet (en haut à gauche) et choisissez **exporter l’audio…**. Les valeurs par défaut sont prêtes : tout le projet en WAV 24 bits à la fréquence d’échantillonnage du moteur. Cliquez sur **exporter**.

Les options :

| Option | Choix | Par défaut |
|---|---|---|
| **plage** | tout le projet, zone de boucle, clips sélectionnés | tout le projet |
| **format** | WAV, FLAC, MP3 (320 kbit/s) | WAV |
| **résolution** | 16 bits (avec dither), 24 bits, 32 bits flottant | 24 bits |
| **fréquence d’échantillonnage** | fréquence du moteur, 44,1, 48, 88,2 ou 96 kHz | fréquence du moteur |
| **queue** | de 0 à 60 secondes | 0 |
| **nom du fichier** | n’importe quel nom | le nom du projet |
| **normaliser** | activé ou désactivé | désactivé |
| **stems** | activé ou désactivé | désactivé |

### plage {#range}

- **tout le projet** : du début jusqu’à la fin du dernier clip (ou jusqu’au dernier point d’automation d’une piste, s’il vient plus tard).
- **zone de boucle** : seulement la partie entre les deux extrémités de l’accolade de boucle.
- **clips sélectionnés** : la sélection temporelle, si vous en avez fait une ([[key:Shift]]-glisser dans l’arrangement), sinon l’intervalle qui va du premier clip sélectionné à la fin du dernier. L’option est grisée tant que rien n’est sélectionné.

### queue {#tail}

L’export s’arrête exactement à la fin de la plage, ce qui coupe la réverbération et le delay qui résonnent encore. Réglez la **queue** sur quelques secondes pour les laisser s’éteindre.

### format, résolution et fréquence d’échantillonnage {#format-bit-depth-and-sample-rate}

- **WAV** accepte toutes les résolutions. Le 16 bits reçoit un dither.
- **FLAC** est sans perte et plus léger que le WAV, en 16 ou 24 bits seulement.
- **MP3** est toujours à 320 kbit/s et n’a pas de résolution. Il ne peut être qu’en 44,1 ou 48 kHz (ou à la fréquence du moteur).

Les choix qui ne conviennent pas au format sont grisés. **fréquence du moteur** exporte à la fréquence d’échantillonnage de votre périphérique audio.

### normaliser {#normalize}

**normaliser** monte ou baisse tout le fichier pour que sa crête la plus forte arrive à -0,1 dBFS. C’est une normalisation de crête, pas une égalisation de loudness : elle ne fait pas sonner un morceau calme aussi fort qu’un master commercial. Avec les stems, chaque stem est normalisé séparément.

## stems {#stems}

Activez **stems** pour exporter un fichier par piste au lieu d’un seul mix. Choisissez les pistes dans la liste : les pistes audio et MIDI sont cochées par défaut. Chaque stem contient :

- le son propre de la piste à travers ses appareils, son volume et son panoramique,
- plus ce qu’elle envoie aux pistes de retour.

Le solo est ignoré, et une piste muette est réactivée pour son propre stem. Une piste à l’intérieur d’un groupe ne passe pas par les appareils du groupe. Le stem d’un groupe inclut ses pistes. Les appareils de la piste master sont exclus de tous les stems.

### quand les stems se chevauchent {#when-stems-overlap}

Si vous choisissez une piste avec son groupe, ou avec un retour vers lequel elle envoie, cet audio se retrouve dans les deux fichiers. Un sidechain qui écoute une autre piste n’entend que du silence dans un stem. Les stems ne s’additionnent donc exactement au mix que si le master et les groupes n’ont aucun traitement qui modifie le son, et qu’il n’y a pas de sidechain.

## pendant l’export {#while-it-exports}

Une barre de progression montre l’avancement du rendu. **annuler l’export** l’arrête. Vous pouvez fermer la fenêtre et continuer à travailler : la barre du haut affiche **export n %**, et un clic dessus rouvre la fenêtre.

## où vont les fichiers {#where-the-files-go}

- **Bureau** [[desktop]] : les fichiers sont enregistrés dans le dossier `exports` du dossier du projet, et la fenêtre les liste à la fin. Un fichier existant n’est jamais écrasé : le nouveau reçoit un numéro, comme « my song (2).wav ».
- **Web** [[web]] : chaque fichier se télécharge par votre navigateur. Tant que vous ne fermez pas la fenêtre, un bouton **télécharger à nouveau** se trouve à côté de chaque fichier.

### les gros exports dans le navigateur {#large-exports-in-the-browser}

Dans le navigateur, les fichiers exportés sont gardés en mémoire jusqu’à leur téléchargement. Au-delà d’environ 200 Mo, la fenêtre vous avertit. Les longs exports de stems à une fréquence d’échantillonnage élevée peuvent atteindre cette taille. La normalisation garde aussi tout le rendu en mémoire, sur toutes les plateformes.

### quand un plug-in arrête l’export {#when-a-plugin-stops-the-export}

L’export fait le rendu hors ligne, avec de nouvelles copies de vos plug-ins chargées avec leurs réglages actuels. Si un plug-in ne peut pas être chargé de cette façon, l’export s’arrête avec une erreur au lieu de laisser discrètement le plug-in de côté.

## et ensuite {#where-next}

- [Le partage](/docs/sharing/overview) : partagez un lien vers votre morceau plutôt qu’un fichier.
- [Exporter vers d’autres DAW](/docs/import-export/exporting-to-other-daws) : envoyez le projet entier.
- [Le mixage](/docs/making-music/mixing) : trouvez d’abord le bon équilibre.
