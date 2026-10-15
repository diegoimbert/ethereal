---
title: confidentialité et télémétrie
order: 40
summary: Ce qu’Ethereal envoie, ce qu’il n’envoie jamais, et comment désactiver chaque partie.
platform: all
audience: everyone
covers:
  - first-run/privacy-notice-on-first-run #the-first-run-notice
  - app-shell/telemetry-is-on-by-default-and-how-to-turn-it-off #crash-reports-and-usage-statistics
---

Ethereal envoie quelques types de données, chacun pour une raison précise et chacun avec son propre interrupteur. Cette page les liste, avec leur contenu et la façon de les désactiver. La liste technique complète se trouve dans [PRIVACY.md](https://github.com/diegoimbert/ethereal/blob/main/docs/PRIVACY.md).

## ce qui n’est jamais envoyé {#what-is-never-sent}

- Vos projets : aucun fichier de projet, aucun nom de piste ou de clip, aucune note ni automation.
- De l’audio, quel qu’il soit.
- Les noms et chemins de fichiers. Votre dossier personnel est retiré des messages d’erreur et des journaux.
- Votre nom, votre adresse e-mail (sauf si vous la tapez dans un signalement de problème) ou votre adresse IP.
- Les liens de partage, les clés d’invitation et les identifiants de salle.

## l’avis au premier lancement {#the-first-run-notice}

La première fois qu’Ethereal se lance, une carte en bas à gauche, **aidez à améliorer ethereal**, vous indique que les rapports de plantage et les statistiques d’utilisation anonymes sont activés. Elle propose un lien **en savoir plus**, **réglages** (ouvre les réglages de confidentialité) et **ok**. L’un ou l’autre bouton, ou le ×, la ferme définitivement. Elle ne bloque rien : vous pouvez continuer à travailler pendant qu’elle est là. Elle n’apparaît que si quelque chose serait réellement envoyé.

## rapports de plantage et statistiques d’utilisation {#crash-reports-and-usage-statistics}

Les deux sont **activés par défaut**. Désactivez chacun dans **réglages › confidentialité** :

- **Envoyer les rapports de plantage** : quand le moteur, l’application ou un plug-in isolé plante, le message d’erreur et l’endroit du code d’Ethereal où c’est arrivé. Pour le plantage d’un plug-in, l’identifiant, le format et la version du plug-in.
- **Envoyer des statistiques d’utilisation anonymes** : des compteurs, envoyés par lots, comme « une session a démarré », « un projet de 9 à 32 pistes a été chargé », « un export en WAV a réussi » ou « le moteur s’est bloqué à cause d’un plug-in ». Chacun est un nom, quelques courtes étiquettes et un nombre.

Désactiver un interrupteur supprime aussi tout ce qui, de ce type, attendait encore d’être envoyé.

Pour nous, vous êtes un **identifiant d’installation** aléatoire, pas un compte ni une personne. **Réglages › confidentialité › réinitialiser l’identifiant d’installation** le remplace : rien de ce qui a été envoyé avant ne peut alors être relié à ce que vous enverrez ensuite. Chaque envoi contient aussi la version de l’application, la famille du système d’exploitation, le type de processeur et s’il s’agit de l’application de bureau ou de l’application web.

### voir exactement ce qui est envoyé {#see-exactly-whats-sent}

**Réglages › confidentialité › voir ce qui est envoyé** affiche les 50 derniers envois exactement tels qu’ils ont quitté cet appareil, du plus récent au plus ancien. La liste reste sur votre appareil.

### quand rien n’est envoyé du tout {#when-nothing-is-sent-at-all}

- [[web]] Quand votre navigateur a **Do Not Track** ou **Global Privacy Control** activé.
- [[desktop]] Quand `DO_NOT_TRACK=1` est défini dans l’environnement.
- Dans les versions de développement.

Quand la télémétrie est désactivée pour l’une de ces raisons, les réglages de confidentialité en indiquent la cause.

## nouveautés {#news}

Quelques secondes après le lancement, Ethereal peut afficher une annonce de l’auteur dans une fenêtre. Pour choisir les bonnes annonces, il envoie seulement la version de l’application, bureau ou web, et la famille de votre système d’exploitation : aucun identifiant d’installation et rien sur votre projet. Le service ne garde rien sur qui a demandé. Désactivez-le avec **réglages › confidentialité › rechercher des nouveautés**. Voir [signaler un problème](/docs/reference/reporting-issues#news) pour le fonctionnement des fenêtres de nouveautés.

## ce que vous envoyez vous-même {#things-you-send-yourself}

Ces données ne sont envoyées que si vous le demandez, et vous voyez d’abord ce qu’elles contiennent :

- **Signaler un problème** : votre description, la catégorie, votre e-mail si vous en tapez un, et (sauf si vous décochez **joindre les diagnostics**) une courte liste sur votre configuration. Voir [ce que joint un signalement](/docs/reference/reporting-issues#what-a-report-attaches).
- **Envoyer les diagnostics…** : les informations système, un résumé du projet ouvert et, sur le bureau, la fin du journal du moteur. Voir [envoyer les diagnostics](/docs/reference/reporting-issues#send-diagnostics).

Do Not Track ne les bloque pas : vous les envoyez volontairement.

## autres services {#other-services}

- **Assistant IA** : si vous configurez un fournisseur d’IA avec votre propre clé, vos messages partent chez ce fournisseur avec un aperçu du projet (noms des pistes, clips, appareils et sections, tempo, mixage) et tout ce que l’assistant lit avec ses outils, comme les notes et les paramètres. Pas d’audio. Il vous demande votre accord avant le premier message à chaque fournisseur. La conversation est gardée sur votre ordinateur avec le projet jusqu’à ce que vous l’effaciez. Voir [l’assistant IA](/docs/advanced/ai-assistant).
- **Partage** : dans une session en direct, le projet, la discussion et les modifications circulent chiffrés directement entre les ordinateurs. Le service de partage ne fait que les mettre en relation et ne voit jamais les noms ni le contenu. Les personnes d’une session peuvent voir l’adresse IP des autres. Voir [sessions en direct](/docs/sharing/live-sessions).
- **Mises à jour** [[desktop]] : l’application de bureau recherche les nouvelles versions. Voir [réglages](/docs/advanced/settings).

## où vont les données {#where-the-data-goes}

Les rapports de plantage, les statistiques, les diagnostics et les signalements de problèmes vont à un service géré par les mainteneurs d’Ethereal. Eux seuls peuvent les lire, et rien n’est vendu ni partagé. Les diagnostics sont supprimés au bout de 90 jours et les signalements au bout d’un an. Les détails, dont la durée de conservation de chaque type, sont dans [PRIVACY.md](https://github.com/diegoimbert/ethereal/blob/main/docs/PRIVACY.md).

## et ensuite {#where-next}

- [Signaler un problème](/docs/reference/reporting-issues).
- [Référence des réglages](/docs/advanced/settings).
- [FAQ](/docs/reference/faq).
