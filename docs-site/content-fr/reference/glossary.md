---
title: glossaire
order: 20
summary: De courtes définitions des mots qu’utilise Ethereal, chacune avec un lien vers la page qui l’explique.
platform: all
audience: everyone
---

Voici les mots que vous rencontrerez dans l’application et dans cette documentation. Chaque entrée est courte et renvoie à la page qui l’explique en détail. Inutile de lire cette page de haut en bas : servez-vous-en comme d’un aide-mémoire, ou cherchez un mot.

## lecture et temps {#playing-and-time}

- **Tête de lecture** : la ligne qui avance pendant la lecture du morceau. Elle montre ce que vous entendez à cet instant. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).
- **Marqueur d’insertion** : la ligne clignotante que laisse un clic sur une piste. Coller, scinder et insérer se font là, pas à la tête de lecture. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).
- **Sélection temporelle** : une plage de temps que vous sélectionnez en faisant glisser avec [[key:Shift]] sur les pistes. Quand il y en a une, les touches d’édition comme copier et supprimer agissent sur cette plage de temps plutôt que sur les clips. Voir [arrangement](/docs/making-music/arranging).
- **Accolade de boucle** : la barre au-dessus de la règle qui délimite la partie du morceau répétée quand la boucle est activée. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).
- **Grille et magnétisme** : les lignes de temps sur lesquelles les clips, les notes et les points se calent pendant que vous les faites glisser. Maintenez [[key:Alt]] pour les placer librement. Voir [arrangement](/docs/making-music/arranging).
- **Marqueur (locator)** : une position nommée sur le couloir des marqueurs, à laquelle vous pouvez sauter. Voir [arrangement](/docs/making-music/arranging).
- **Tempo et signature rythmique** : la vitesse du morceau en bpm, et la façon dont les temps sont groupés en mesures. Les deux peuvent changer au cours du morceau. Voir [tempo, signature rythmique et gamme](/docs/basics/tempo-and-time-signature).
- **Décompte** : des mesures de clics du métronome avant le début de l’enregistrement. Voir [enregistrement](/docs/making-music/recording).
- **Punch in/out** : n’enregistrer qu’à l’intérieur de la zone de boucle. Voir [enregistrement](/docs/making-music/recording).

## pistes et clips {#tracks-and-clips}

- **Piste** : une rangée de l’arrangement. Les pistes audio contiennent des enregistrements et des échantillons, les pistes MIDI contiennent des notes et jouent un instrument. Voir [pistes](/docs/making-music/tracks).
- **Piste de groupe** : une piste qui contient d’autres pistes et les mixe ensemble, pour les replier, les rendre muettes ou les traiter comme une seule. Voir [pistes](/docs/making-music/tracks).
- **Piste de retour et envoi** : un envoi transmet une partie du son d’une piste à une piste de retour, en général pour partager une même reverb ou un même delay entre plusieurs pistes. Voir [mixage](/docs/making-music/mixing).
- **Piste VCA** : un fader qui monte ou baisse le volume des pistes qui lui sont assignées, sans faire passer leur son par lui. Voir [pistes](/docs/making-music/tracks).
- **Piste master** : là où aboutissent toutes les pistes avant vos enceintes ou un export. Voir [mixage](/docs/making-music/mixing).
- **Armer** : préparer une piste à l’enregistrement (le bouton ● sur son en-tête). Voir [enregistrement](/docs/making-music/recording).
- **Monitoring** : entendre une entrée en direct à travers les appareils d’une piste. Voir [enregistrement](/docs/making-music/recording).
- **Clip** : un bloc d’audio ou de notes sur une piste. Voir [clips](/docs/making-music/clips).
- **Édition par glissement du contenu (slip)** : déplacer le contenu d’un clip à l’intérieur de celui-ci, sans bouger le clip. Voir [clips](/docs/making-music/clips).
- **Fondu et fondu enchaîné** : une courte rampe de volume au bord d’un clip, ou entre deux clips qui se touchent. Voir [clips audio](/docs/making-music/audio-clips).
- **Warp** : faire suivre le tempo du projet à un clip audio. **Repitch** change la vitesse et la hauteur ensemble, **complex** garde la hauteur. Les **marqueurs de warp** attachent des instants de l’audio à des temps. Voir [clips audio](/docs/making-music/audio-clips).
- **Prise** : un passage d’enregistrement. Enregistrer de nouveau au même endroit crée d’autres prises au lieu de les mélanger. Voir [prises et comping](/docs/making-music/takes-and-comping).
- **Comp (comping)** : construire une seule interprétation à partir des meilleurs passages de plusieurs prises. Voir [prises et comping](/docs/making-music/takes-and-comping).
- **Piano roll** : l’éditeur de notes d’un clip MIDI. Voir [le piano roll](/docs/making-music/piano-roll).
- **Vélocité** : la force avec laquelle une note est jouée, de 1 à 127. Voir [le piano roll](/docs/making-music/piano-roll).
- **Quantifier** : placer les notes sur la grille. Voir [le piano roll](/docs/making-music/piano-roll).
- **Groove, swing et humanisation** : décaler volontairement les notes de la grille, selon un motif (swing) ou au hasard (humanisation). Voir [groove, swing et humanisation](/docs/advanced/groove).
- **Expression et MPE** : des contrôleurs, du pitch bend et de la pression dessinés sous forme de courbes. Le MPE donne à chaque note ses propres courbes. Voir [expression et MPE](/docs/making-music/expression-and-mpe).
- **Capturer le MIDI** : transformer ce que vous venez de jouer en clip, même si vous n’enregistriez pas. Voir [capturer le MIDI](/docs/making-music/capture-midi).

## appareils et son {#devices-and-sound}

- **Appareil** : un instrument ou un effet sur une piste. Les appareils intégrés fonctionnent dans toutes les versions d’Ethereal ; les plug-ins demandent l’application de bureau. Voir [appareils et presets](/docs/making-music/devices-and-presets).
- **Chaîne d’appareils** : les appareils d’une piste, dans l’ordre. Le son les traverse de gauche à droite. Voir [appareils et presets](/docs/making-music/devices-and-presets).
- **Instrument** : un appareil qui transforme des notes en son, sur une piste MIDI. Voir [instruments](/docs/making-music/instruments).
- **Effet** : un appareil qui modifie le son (effet audio) ou les notes (effet MIDI). Voir [effets](/docs/making-music/effects).
- **Preset** : des réglages enregistrés pour un appareil. Les presets d’usine sont fournis avec Ethereal, les vôtres vivent dans votre bibliothèque utilisateur. Voir [appareils et presets](/docs/making-music/devices-and-presets).
- **Bibliothèque utilisateur** : vos propres presets et fichiers, affichés dans la bibliothèque à côté du contenu d’usine. Voir [le navigateur](/docs/sounds-and-plugins/browser).
- **Bibliothèque (navigateur)** : le panneau où vous trouvez échantillons, presets, appareils et plug-ins, et d’où vous les faites glisser dans le projet. Voir [le navigateur](/docs/sounds-and-plugins/browser).
- **Plug-in** : un instrument ou un effet d’un autre éditeur (CLAP, VST3 ou AU). Bureau uniquement. Voir [plug-ins](/docs/sounds-and-plugins/plugins).
- **Plug-in isolé** : un plug-in qui tourne dans son propre processus, pour qu’un plantage n’arrête pas l’application. Voir [plug-ins](/docs/sounds-and-plugins/plugins).
- **Drum rack** : un instrument avec un pad par son, chacun avec sa propre chaîne. Voir [le drum rack](/docs/making-music/drum-rack).
- **Groupe d’étouffement** : des pads qui se coupent les uns les autres, comme un charleston ouvert et un charleston fermé. Voir [le drum rack](/docs/making-music/drum-rack).
- **Rack** : un appareil qui contient plusieurs chaînes côte à côte. Voir [racks et macros](/docs/making-music/racks-and-macros).
- **Chaîne** : un chemin à travers un rack, avec ses propres appareils. Voir [racks et macros](/docs/making-music/racks-and-macros).
- **Macro** : un bouton d’un rack qui tourne plusieurs paramètres à l’intérieur à la fois. Voir [racks et macros](/docs/making-music/racks-and-macros).
- **Modulateur** : une source comme un LFO ou une enveloppe qui fait bouger des paramètres tout seul. Voir [modulation et sidechain](/docs/making-music/modulation-and-sidechain).
- **Sidechain** : laisser le son d’une piste piloter un effet sur une autre, par exemple une grosse caisse qui fait baisser une basse. Voir [modulation et sidechain](/docs/making-music/modulation-and-sidechain).
- **Automation** : des changements de paramètres dessinés dans le temps dans un couloir. Voir [automation](/docs/making-music/automation).
- **Enveloppe de clip** : une automation qui vit à l’intérieur d’un clip et se déplace avec lui. Voir [automation](/docs/making-music/automation).
- **Apprentissage MIDI (assignation MIDI)** : relier un bouton de votre contrôleur MIDI à une commande d’Ethereal. Voir [apprentissage MIDI](/docs/advanced/midi-learn).
- **Clavier MIDI de l’ordinateur** : jouer des notes avec le clavier de votre ordinateur. Voir [le clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard).

## rendu {#rendering}

- **Geler** : faire le rendu d’une piste en audio pour que ses appareils n’utilisent plus le processeur. Une piste gelée est verrouillée jusqu’à ce que vous la dégeliez. Voir [geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce).
- **Aplatir** : remplacer les clips et appareils d’une piste gelée par un seul clip audio du rendu. Seule l’annulation les fait revenir. Voir [geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce).
- **Rendu et consolidation** : faire le rendu d’une partie du morceau dans un nouveau clip audio. Voir [geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce).
- **Stems** : un fichier audio par piste choisie, chacun exporté séparément. Voir [exporter l’audio](/docs/import-export/exporting-audio).
- **Coupure (xrun)** : un clic ou un trou dans le son quand l’ordinateur ne parvient pas à remplir le buffer audio à temps. Voir [dépannage](/docs/reference/troubleshooting).
- **Taille du buffer et latence** : la quantité d’audio calculée d’un coup. Un buffer plus petit veut dire moins de retard mais plus de risque de coupures. Voir [configuration audio et MIDI](/docs/making-music/audio-and-midi-setup).

## projets {#projects}

- **Projet** : un morceau, avec ses pistes, clips, appareils et médias. Voir [projets et enregistrement](/docs/basics/projects).
- **Brouillon** : le projet que vous obtenez en fermant l’écran d’accueil. Si vous n’y touchez pas, il est jeté. Voir [projets et enregistrement](/docs/basics/projects).
- **Modèle** : un projet dont partent les nouveaux projets. Voir [projets et enregistrement](/docs/basics/projects).
- **Versions** : des sauvegardes automatiques d’un projet auxquelles vous pouvez revenir. Un **instantané** est une version que vous enregistrez exprès, avec un nom. Voir [versions et sauvegardes](/docs/basics/versions-and-backups).
- **Point de repère** : une étape nommée dans l’historique d’annulation. Voir [annuler et historique](/docs/basics/undo-and-history).
- **Fichier .ether** : un projet entier dans un seul fichier, pour le déplacer vers un autre ordinateur ou en garder une sauvegarde. Voir [déplacer des projets entre ordinateurs](/docs/basics/moving-projects).
- **Médias** : les fichiers audio qu’utilise un projet. Voir [déplacer des projets entre ordinateurs](/docs/basics/moving-projects).
- **Mode sans échec** : ouvrir un projet avec tous les plug-ins désactivés, pour contourner un plug-in qui plante. Voir [mode sans échec](/docs/advanced/safe-mode).
- **Palette de commandes** : le champ de recherche de toutes les commandes, qui s’ouvre avec [[action:palette.open]]. Voir [la palette de commandes et les réglages](/docs/basics/command-palette-and-settings).

## partage {#sharing}

- **Session en direct** : travailler ensemble sur un projet, en temps réel. Voir [sessions en direct](/docs/sharing/live-sessions).
- **Hôte** : la personne qui a partagé le projet. Son ordinateur fait tenir la session. Voir [sessions en direct](/docs/sharing/live-sessions).
- **Lien d’invitation** : le lien qui permet à quelqu’un de rejoindre. Un lien **peut modifier** lui permet de changer le projet ; un lien **peut écouter** lui permet seulement d’écouter. Voir [partager votre musique](/docs/sharing/overview).
- **Copie hors ligne** : la copie d’un projet partagé sur laquelle vous continuez à travailler pendant que l’hôte est hors ligne. Vos modifications se synchronisent au retour de l’hôte. Voir [sessions en direct](/docs/sharing/live-sessions).
- **Copie locale** : un projet séparé nommé « … (copie locale) ». Il est créé quand des modifications faites hors ligne sur un projet partagé ne peuvent pas être fusionnées dans la session. Voir [sessions en direct](/docs/sharing/live-sessions).
- **Jouer avec** : choisir l’ordinateur qui joue le son dans une session. Voir [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together).
- **Lien statique (instantané)** : une copie en lecture seule d’un projet publiée sur le web, pour que d’autres l’écoutent. Voir [liens de partage statiques](/docs/sharing/static-links).
- **Visionneuse de partage** : la page en lecture seule qu’ouvre un lien statique. Voir [la visionneuse de partage](/docs/sharing/share-viewer).

## l’application {#the-app}

- **Moteur** : la partie d’Ethereal qui produit le son. Dans l’application de bureau, il tourne sur votre ordinateur ; dans l’application web, il tourne dans l’onglet de votre navigateur. Voir [qu’est-ce qu’Ethereal](/docs/welcome/what-is-ethereal).
- **Serveur de moteur** : un moteur qui tourne sur une autre machine (`ether-server`) et auquel cette fenêtre se connecte. Voir [le serveur de moteur](/docs/advanced/engine-server).
- **MCP** : le moyen par lequel des agents IA extérieurs peuvent contrôler Ethereal. Voir [agents IA (MCP)](/docs/advanced/mcp).
- **Inspecteur** : le panneau qui affiche les réglages de ce que vous avez sélectionné. Voir [une visite de l’écran](/docs/basics/the-screen).
- **Tiroir de l’éditeur** : le panneau du bas avec les éditeurs de clip (piano roll, warp, automation). [[action:view.editor]] l’affiche ou le masque. Voir [une visite de l’écran](/docs/basics/the-screen).
- **Bêta mobile** : la version tactile d’Ethereal pour téléphones et tablettes. Voir [Ethereal sur votre téléphone](/docs/mobile/mobile-beta).

## et ensuite {#where-next}

- [Dépannage](/docs/reference/troubleshooting) quand quelque chose ne fonctionne pas.
- [FAQ](/docs/reference/faq) pour des réponses rapides.
- [Raccourcis clavier](/docs/reference/keyboard-shortcuts).
