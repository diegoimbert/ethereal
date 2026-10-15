---
title: signaler un problème
order: 50
summary: Envoyez un rapport de bug ou une idée depuis l’application, ajoutez des diagnostics quand on vous les demande, et retrouvez les nouveautés passées.
platform: all
audience: everyone
covers:
  - app-shell/report-an-issue #report-an-issue
  - app-shell/what-an-issue-report-attaches #what-a-report-attaches
  - app-shell/send-diagnostics-ticket-for-a-bug #send-diagnostics
  - app-shell/debug-panels-advanced #debug-panels
  - first-run/news-pop-up-after-launch #news
---

Quelque chose ne marche plus, il manque une fonction, ou vous avez une idée ? Dites-le-nous depuis l’application. Vous n’avez pas besoin de compte. Avant d’écrire, un coup d’œil au [dépannage](/docs/reference/troubleshooting) vous donnera peut-être déjà la réponse.

## signaler un problème {#report-an-issue}

1. Cliquez sur **signaler un problème** à l’extrémité droite de la fine barre d’état en bas de la fenêtre. [[mobile]] Sur la version mobile, c’est dans le menu du projet.
2. Choisissez une catégorie : **bug**, **fonction manquante**, **idée / suggestion**, **performances**, **plantage** ou **autre**.
3. Décrivez ce qui s’est passé dans **que s’est-il passé ?**. C’est le seul champ obligatoire, jusqu’à 4000 caractères. Pour un bug, dites ce que vous avez fait, ce que vous attendiez et ce qui s’est passé à la place.
4. Ajoutez votre e-mail si vous souhaitez une réponse. Laissez-le vide pour rester anonyme.
5. Appuyez sur **envoyer** (ou [[key:Mod+Enter]] dans la description).

Vous recevez un identifiant de signalement qui commence par `R-`. Gardez-le si vous voulez faire référence à votre signalement plus tard. Si l’envoi échoue, le formulaire garde ce que vous avez tapé et propose **réessayer**. Vous pouvez envoyer un nombre limité de signalements par heure ; si vous atteignez la limite, réessayez plus tard.

Si le moteur audio ne démarre pas, l’écran d’erreur a son propre bouton **signaler un problème**.

## ce que joint un signalement {#what-a-report-attaches}

**Joindre les diagnostics** est activé par défaut. Ouvrez **ce qui est joint** dans le formulaire pour voir la liste exacte avant d’envoyer. Elle contient :

- la version de l’application, et s’il s’agit de l’application de bureau ou de l’application web ;
- votre système d’exploitation, le nombre de processeurs, la mémoire et la langue ;
- la configuration audio : backend, fréquence d’échantillonnage, taille du buffer, latence, nombre de coupures, et si le moteur tourne ;
- la taille du projet ouvert : le nombre de pistes, de clips, de notes, d’appareils et de fichiers médias.

Elle ne contient jamais le projet lui-même, ni noms, ni chemins, ni clés. Désactivez **joindre les diagnostics** pour n’envoyer que votre texte. Do Not Track ne bloque pas un signalement, car vous l’envoyez volontairement. Voir [confidentialité et télémétrie](/docs/reference/privacy-and-telemetry).

## envoyer les diagnostics {#send-diagnostics}

Quand nous vous demandons plus de détails sur un bug, utilisez **envoyer les diagnostics…** : dans la palette de commandes ([[action:palette.open]], puis tapez « diagnostics »), ou dans **réglages › confidentialité › envoyer les diagnostics…**.

La fenêtre vous permet d’ajouter une description facultative et affiche **ce qui sera envoyé** avant que vous l’envoyiez :

- les informations système : plateforme, processeurs, mémoire, langue, backend audio, fréquence d’échantillonnage, taille du buffer et coupures ;
- un résumé du projet ouvert : les nombres d’éléments, et les noms, formats et versions des plug-ins qu’il utilise ;
- [[desktop]] la fin du [journal du moteur](/docs/reference/troubleshooting#the-engine-log).

Les chemins, e-mails et adresses IP sont retirés. Vous recevez un identifiant de ticket comme `D-7K3QX9`, avec un bouton **copier** : citez-le dans votre signalement. Les diagnostics sont conservés 90 jours. Il faut que la télémétrie soit disponible : si elle est désactivée (par exemple avec Do Not Track), la fenêtre indique que les diagnostics ne peuvent pas être envoyés d’ici.

## panneaux de débogage {#debug-panels}

Les panneaux de débogage sont des fenêtres flottantes qui montrent ce que fait une partie de l’application en interne, pour les développeurs. Quand il y en a, ils sont listés dans **réglages › avancé › panneaux de débogage** et dans la palette de commandes sous **debug : …**, et Ethereal se souvient de ceux qui sont ouverts et de leur position. Il n’y en a aucun dans la version actuelle : vous ne verrez donc pas cette section.

## nouveautés {#news}

Environ quatre secondes après le lancement, Ethereal peut afficher une annonce de son auteur dans une fenêtre. Fermez-la comme vous voulez (**fermer**, le ×, [[key:Escape]]) et elle ne réapparaîtra plus. S’il y en a plusieurs, **suivante** passe à la suivante et marque l’actuelle comme vue.

Pour relire les annonces passées, choisissez **nouveautés…** dans le menu du projet ou la palette de commandes. Pour ne plus voir ces fenêtres, désactivez **réglages › confidentialité › rechercher des nouveautés** : Ethereal ne demande alors plus du tout les nouveautés. Voir [confidentialité et télémétrie](/docs/reference/privacy-and-telemetry#news).

## et ensuite {#where-next}

- [Dépannage](/docs/reference/troubleshooting).
- [Confidentialité et télémétrie](/docs/reference/privacy-and-telemetry).
- [FAQ](/docs/reference/faq).
