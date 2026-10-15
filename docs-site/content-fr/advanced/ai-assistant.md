---
title: l’assistant ia
order: 10
summary: Demandez à un modèle d’IA de modifier votre projet, avec votre propre clé d’API ou un modèle sur votre ordinateur.
platform: all
audience: advanced
covers:
  - ai-mcp/ask-ai-panel #the-ask-ai-panel
  - ai-mcp/bring-your-own-api-key #connecting-a-provider
  - ai-mcp/where-your-key-is-stored #where-your-key-is-stored
  - ai-mcp/what-is-sent-to-the-ai-provider #what-is-sent-to-the-provider
  - ai-mcp/each-ai-action-is-one-undo-step #every-ai-edit-is-one-undo-step
  - ai-mcp/tool-calls-stop-and-round-limit #tool-calls-stop-and-the-round-limit
  - ai-mcp/switching-provider-starts-fresh #the-saved-conversation-and-switching-provider
  - ai-mcp/web-cors-blocks-some-providers #providers-blocked-in-the-browser
  - ai-mcp/models-without-tool-support #models-without-tool-support
  - ai-mcp/what-the-ai-can-t-do #what-the-ai-can-and-cant-do
---

Ethereal dispose d’un panneau de chat où vous décrivez une modification avec des mots simples (« ajoute un arpège de do mineur sur une nouvelle piste », « règle le tempo sur 92 ») et un modèle d’IA l’effectue dans votre projet ouvert. Il vous faut une clé d’API d’un fournisseur d’IA, ou un modèle qui tourne sur votre propre ordinateur. Cette page explique comment le configurer, ce que voit l’IA et comment annuler ce qu’elle fait.

> [!note]
> L’assistant IA est en **alpha** : il peut faire des erreurs et reste limité pour l’instant, et il sera largement amélioré à l’avenir. Le panneau affiche un badge **alpha**, et le précise dans la carte de première utilisation et au-dessus de vos conversations (vous pouvez masquer cette ligne ; le badge reste).

## le panneau demander à l’ia {#the-ask-ai-panel}

Ouvrez l’onglet **demander à l’IA** dans la barre latérale gauche, appuyez sur [[action:ai:ask]], ou lancez **demander à l’IA** depuis la palette de commandes ([[action:palette.open]]). Tapez une demande et appuyez sur [[key:Enter]] pour l’envoyer ([[key:Shift+Enter]] ajoute un saut de ligne). Un chat vide suggère quelques demandes à essayer.

L’IA peut créer et renommer des pistes, écrire des notes MIDI, ajouter des appareils et régler leurs paramètres, régler le volume et le panoramique, changer le tempo, contrôler le transport et exporter l’audio. Vous voyez les modifications apparaître dans l’arrangement pendant qu’elle travaille. Un projet doit d’abord être ouvert.

## connecter un fournisseur {#connecting-a-provider}

La première fois que vous ouvrez le panneau, il vous demande de choisir un fournisseur et de coller une clé :

- Fournisseurs cloud : Anthropic (Claude, par défaut), OpenAI, DeepSeek, Google Gemini, Mistral, Groq, OpenRouter et Together AI.
- Modèles locaux : Ollama et LM Studio, qui tournent sur votre ordinateur. Ils ne demandent pas de clé.
- **autre (compatible OpenAI)** : tout serveur qui parle l’API de chat d’OpenAI.

Vous utilisez votre propre clé, et le fournisseur vous facture ce que vous consommez. Ethereal ne revend pas d’accès. Pour changer plus tard de fournisseur, de modèle, d’URL de base ou de clé, cliquez sur le bouton **réglages de l’IA** (la roue dentée) dans l’en-tête du panneau. **tester la connexion** vérifie la clé et le modèle avec une requête gratuite. Le sélecteur de modèle dans l’en-tête du panneau permet de changer rapidement de modèle.

### où est stockée votre clé {#where-your-key-is-stored}

- [[desktop]] Dans les données de l’application Ethereal, sur cet ordinateur.
- [[web]] Dans le stockage local de ce navigateur. Toute personne qui utilise le même profil de navigateur peut la lire.

Dans les deux cas, la clé n’est jamais envoyée qu’au fournisseur que vous avez choisi. Elle n’est envoyée ni au moteur, ni aux participants d’une session en direct, ni aux serveurs d’Ethereal.

## ce qui est envoyé au fournisseur {#what-is-sent-to-the-provider}

Chaque message part chez le fournisseur avec un aperçu de votre projet : les noms des pistes, des clips, des appareils et des marqueurs, le mix et le tempo. Tout ce que le modèle lit avec ses outils (notes, valeurs de paramètres) est envoyé aussi. Les fichiers audio ne sont jamais envoyés.

La première fois que vous utilisez un fournisseur, le panneau indique ce qu’il envoie (le contenu du projet : pistes, clips, notes, réglages des appareils) et à qui, et attend votre accord avec **envoyer à …**. Il le demande une fois par fournisseur et retient votre réponse sur cet ordinateur. Passer à un autre fournisseur le redemande.

> [!warning]
> Une fois que vous avez accepté, chaque message envoie les noms et le contenu de votre projet à ce fournisseur. Ne l’utilisez pas sur un projet que vous ne pouvez pas partager avec ce fournisseur, ou utilisez un modèle local.

## chaque modification de l’ia est une étape d’annulation {#every-ai-edit-is-one-undo-step}

Chaque changement de l’IA est appliqué immédiatement comme une étape d’annulation, nommée **IA : …** dans l’historique (par exemple **IA : ajouter des notes**). Appuyez sur [[action:edit.undo]] pour annuler la dernière, ou revenez en arrière dans le panneau **historique**. Dans une session en direct, les modifications de l’IA arrivent chez tout le monde et comptent comme les vôtres : votre annulation les annule, celle des autres non.

## appels d’outils, arrêt et limite de tours {#tool-calls-stop-and-the-round-limit}

Chaque action de l’IA apparaît comme une ligne dans le chat. Dépliez-la pour voir ce que l’IA a envoyé et ce qui est revenu, erreurs comprises.

- **arrêter** (ou [[key:Escape]] dans le champ du message) interrompt l’IA. Les modifications déjà faites restent, et vous pouvez les annuler.
- Une réponse se met en pause après 25 tours d’appels d’outils, pour qu’une demande qui s’emballe ne tourne pas indéfiniment. Dites « continue » pour la laisser poursuivre. Changez la limite (de 1 à 200) sous **tours d’outils par message** dans les réglages de l’IA.

## la conversation enregistrée et le changement de fournisseur {#the-saved-conversation-and-switching-provider}

La conversation appartient au projet. Elle est gardée sur cet ordinateur (pas dans le fichier du projet, donc elle ne voyage pas avec le projet et n’atteint pas les participants d’une session en direct) et revient quand vous rouvrez le projet ou rechargez l’application, et l’IA reprend là où elle s’était arrêtée. Chaque projet a sa propre conversation.

**effacer le chat** (le bouton gomme) vide la conversation et oublie celle qui était enregistrée. Une conversation très longue ne garde que sa partie la plus récente.

Si vous passez à un autre fournisseur au milieu d’une conversation, le message suivant démarre une nouvelle conversation avec le nouveau fournisseur. Le panneau le signale, car le nouveau modèle ne voit pas les messages précédents.

## fournisseurs bloqués dans le navigateur [[web]] {#providers-blocked-in-the-browser}

Dans la version web, certains fournisseurs refusent les requêtes qui viennent directement d’une page web (CORS). Dans ce cas, le panneau indique que le fournisseur a bloqué la requête et propose une solution :

- Ollama : autorisez l’origine de la page avec le réglage `OLLAMA_ORIGINS`.
- LM Studio : activez **enable cors** dans ses réglages de serveur.
- API cloud : utilisez OpenRouter, ou un proxy.

L’application de bureau n’est pas concernée : elle fait ces requêtes elle-même, hors du navigateur.

## modèles sans prise en charge des outils {#models-without-tool-support}

L’IA modifie votre projet en appelant des outils, donc le modèle doit prendre en charge les appels d’outils. Certains modèles locaux ne le font pas. Quand le serveur le signale, le panneau vous prévient et suggère de choisir un autre modèle.

## ce que l’ia peut et ne peut pas faire {#what-the-ai-can-and-cant-do}

L’IA travaille uniquement avec les outils d’Ethereal : les mêmes modifications que vous pouvez faire dans l’application. Elle ne peut pas lire ni écrire d’autres fichiers sur votre ordinateur, lancer des programmes ou ouvrir d’autres projets.

- Les plug-ins n’existent que sur l’application de bureau, donc dans la version web l’IA ne trouve aucun plug-in à ajouter.
- Chercher dans la bibliothèque de sons demande l’index de la bibliothèque. Sans lui, l’IA ne peut pas charger d’échantillons ni de presets.

Pour laisser une application d’IA extérieure à Ethereal modifier votre projet, voir [agents ia (mcp)](/docs/advanced/mcp).

## et ensuite {#where-next}

- [agents ia (mcp)](/docs/advanced/mcp) : connectez Claude Code ou Claude Desktop à l’application de bureau.
- [annuler et historique](/docs/basics/undo-and-history) : revenez en arrière sur les modifications de l’IA.
- [confidentialité et télémétrie](/docs/reference/privacy-and-telemetry) : ce qu’Ethereal envoie lui-même.
