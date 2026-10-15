---
title: liens de partage statiques
order: 50
summary: Publiez un instantané figé et jouable de votre projet sous forme de lien qui fonctionne même quand vous êtes hors ligne, et contrôlez ce que les gens peuvent récupérer.
platform: all
audience: everyone
covers:
  - sharing/static-snapshot-what-gets-published #what-a-snapshot-contains
  - sharing/static-permissions #permissions
  - sharing/what-a-static-share-always-exposes #what-anyone-with-the-link-can-see
  - sharing/hide-devices #hide-devices
  - sharing/download-can-t-stop-recording #download-and-recording
  - sharing/publishing-again-replaces-the-old-link #publishing-again
  - sharing/snapshots-never-expire-only-this-device-can-remove-them #links-dont-expire
  - sharing/publish-progress-cancel-and-rate-limits #publishing-takes-a-moment
---

Un lien statique est un instantané de votre projet que n’importe qui peut ouvrir dans un navigateur et écouter, même quand votre ordinateur est éteint. Cette page explique ce qui est publié, les permissions, et comment remplacer ou supprimer un lien. Pour travailler ensemble en direct, voir plutôt [sessions en direct](/docs/sharing/live-sessions).

## publier un instantané {#publish-a-snapshot}

1. Cliquez sur **partager** dans la barre du haut et choisissez l’onglet **instantané statique**.
2. Réglez les permissions (voir plus bas).
3. Cliquez sur **publier l’instantané**.
4. Une fois terminé, cliquez sur **copier** et envoyez le lien.

Le lien s’ouvre dans [la visionneuse de partage](/docs/sharing/share-viewer).

## ce que contient un instantané {#what-a-snapshot-contains}

Ethereal fait le rendu du mix (et, par défaut, de chaque piste audio et MIDI séparément) en MP3 à 320 kbit/s, avec une vue en lecture seule du projet, et l’envoie vers un lien de la forme `ethereal.ws/s/…`.

Il est figé : les modifications que vous faites ensuite n’y figurent pas tant que vous ne publiez pas à nouveau.

## permissions {#permissions}

| interrupteur | par défaut | ce qu’il fait |
|---|---|---|
| **autoriser la modification (ils peuvent ouvrir une copie)** | désactivé | les auditeurs ont **ouvrir une copie à modifier** : leur propre copie privée du projet. |
| **autoriser l’écoute des pistes séparément** | activé | les auditeurs peuvent mettre chaque piste en solo ou en muet. Désactivé : seul le mix est rendu et envoyé, la publication est donc plus rapide. |
| **autoriser le téléchargement** | désactivé | les auditeurs peuvent télécharger le mix, chaque piste (si l’interrupteur précédent est activé) et le projet, en FLAC 24 bits sans perte. |
| **masquer les appareils** | désactivé | retire tous les appareils et plug-ins de la vue et de la copie du projet (voir plus bas). |

Les indications sous les interrupteurs disent exactement ce que les gens pourront faire. Changer un interrupteur après la publication transforme le bouton en **publier à nouveau** : les interrupteurs ne s’appliquent qu’au prochain instantané.

## ce que voit quiconque a le lien {#what-anyone-with-the-link-can-see}

Même avec tous les interrupteurs désactivés, la visionneuse montre tout l’arrangement : noms des pistes, clips, notes MIDI, automation, valeurs du mixeur, marqueurs et tempo. À moins de **masquer les appareils**, elle montre aussi vos appareils et plug-ins par leur nom, avec les valeurs de leurs paramètres.

Elle n’inclut jamais l’état des plug-ins, les chemins des fichiers sur votre ordinateur ni la discussion.

## masquer les appareils {#hide-devices}

**masquer les appareils** retire toutes les chaînes d’appareils et de plug-ins (avec leur automation, leurs racks et leurs modulateurs) de la vue et du projet téléchargeable. Les pistes MIDI de cette copie du projet sont alors muettes ; les pistes rendues portent toujours le son.

> [!note]
> Les sessions en direct ont leur propre interrupteur, **masquer les appareils aux auditeurs** : voir [sessions en direct](/docs/sharing/live-sessions#hiding-devices-from-listeners).

## téléchargement et enregistrement {#download-and-recording}

Sans **autoriser le téléchargement**, il n’y a pas de menu de téléchargement et le serveur refuse les fichiers sans perte. Mais n’importe qui peut toujours enregistrer l’audio que joue son navigateur. Ne publiez pas de contenu dont la fuite vous poserait problème.

## publier à nouveau {#publishing-again}

**publier à nouveau** (ou **publier un nouvel instantané**) envoie un nouvel instantané avec un **nouveau lien**, puis supprime l’ancien : le lien que vous aviez envoyé cesse de fonctionner. Faites circuler le nouveau lien.

**dépublier** supprime l’instantané immédiatement ; son lien cesse de fonctionner.

## les liens n’expirent pas {#links-dont-expire}

Un instantané reste en ligne jusqu’à ce que vous le supprimiez. La clé qui vous permet de le remplacer ou de le dépublier n’est stockée que dans cette application, sur cet ordinateur (dans ce navigateur, pour l’application web). Sur un autre ordinateur, ou après avoir effacé les données de votre navigateur, vous ne pouvez plus le dépublier ni le remplacer.

> [!warning]
> Gardez l’application (ou le profil de navigateur) depuis laquelle vous avez publié si vous risquez de vouloir retirer un lien plus tard.

## la publication prend un moment {#publishing-takes-a-moment}

La fenêtre affiche chaque étape : « rendu du mix… », « rendu des pistes… », « empaquetage du projet… », puis « envoi… 3 sur 12 ». Elle peut faire une pause avec « en attente pour continuer l’envoi… » quand le serveur demande de ralentir ; elle reprend d’elle-même.

**annuler** arrête tout et supprime ce qui a déjà été envoyé. Fermer la fenêtre n’annule pas, mais fermer le projet ou en ouvrir un autre, si.

Pour publier sur votre propre serveur de stockage au lieu de celui par défaut, réglez **stockage des partages statiques** dans **réglages › avancé**.

## et ensuite {#where-next}

- [la visionneuse de partage](/docs/sharing/share-viewer) : ce que voient vos auditeurs.
- [exporter l’audio](/docs/import-export/exporting-audio) : faire le rendu de fichiers plutôt qu’un lien.
