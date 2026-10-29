---
title: instruments
order: 60
summary: Ce que fait chaque instrument intégré et ce que changent ses principaux réglages.
platform: all
audience: everyone
covers:
  - devices/synth #synth
  - devices/poly-synth #poly-synth
  - devices/fm-synth #fm-synth
  - devices/sampler #sampler
  - devices/sampler-slice-mode #slice-mode
  - devices/multisampler #multisampler
  - devices/external-instrument-external-audio-effect #external-instrument
---

Un instrument transforme les notes d’une piste MIDI en son. Chaque piste MIDI a un instrument, au milieu de sa chaîne (voir [appareils et presets](/docs/making-music/devices-and-presets)). Cette page passe en revue les instruments intégrés, du plus simple au plus spécialisé. Chacun a des presets d’usine : ouvrez le bouton **presets** de son en-tête pour les essayer avant de tourner les boutons.

| instrument | idéal pour |
|---|---|
| **synthé** | une basse, un lead ou une nappe rapide à partir d’un seul oscillateur simple |
| **synthé poly** | des sons de synthé plus riches : deux oscillateurs, tables d’ondes, unisson, LFO |
| **synthé fm** | cloches, pianos électriques, sons métalliques et cristallins |
| **sampler** | jouer un fichier audio, en one-shot ou sur tout le clavier |
| **multisampler** | de vrais instruments enregistrés note par note |
| **drum rack** | des kits de batterie, un son par pad (voir [le drum rack](/docs/making-music/drum-rack)) |
| **instrument externe** | un synthé matériel branché à votre ordinateur [[desktop]] |

## synthé {#synth}

Un synthé polyphonique simple (jusqu’à 16 notes à la fois) avec un oscillateur.

- **forme d’onde** : sinus, dent de scie, carré ou triangle. La dent de scie et le carré sonnent brillants, le sinus et le triangle doux.
- **transposition** et **fin** : la hauteur en demi-tons et en cents. **plage de bend** règle jusqu’où va un pitch bend complet (2 demi-tons par défaut), depuis la molette d’un clavier MIDI ou le couloir de pitch bend d’un clip.
- **attaque**, **déclin**, **maintien**, **relâchement** : l’enveloppe de volume de chaque note.
- **coupure** et **résonance** : un filtre passe-bas. Baissez la coupure pour un son plus sombre.
- **quantité d’env.** : laisse l’enveloppe de volume balayer la coupure (jusqu’à six octaves dans un sens ou dans l’autre), pour des sons pincés et des attaques façon wah. **suivi de clavier** ouvre davantage le filtre sur les notes aiguës.
- **glissé** : fait glisser la hauteur d’une note à la suivante.
- **vélocité** : à quel point jouer plus fort rend une note plus forte.
- **mode** : **poly** joue des accords ; **mono** joue une note à la fois ; **legato** joue une note à la fois et ne glisse que quand les notes se chevauchent. En mono et en legato, relâcher la dernière touche jouée revient à la dernière touche encore tenue.

## synthé poly {#poly-synth}

Un synthé plus complet avec deux oscillateurs, un sous-oscillateur et du bruit.

- **osc 1** et **osc 2** : dent de scie, carré, triangle, sinus ou **table d’ondes**. Un oscillateur à table d’ondes utilise l’une des 8 tables (formes de base, balayage harmonique, PWM, formant, numérique, orgue, voix, métallique), et **position** la parcourt en fondu. Chaque oscillateur a une octave, des demi-tons, un réglage fin, une largeur d’impulsion et un niveau. L’osc 2 démarre muet : montez son niveau pour l’entendre.
- **sub / bruit** : un sous-oscillateur une ou deux octaves plus bas, et un bruit avec un réglage de couleur. Les deux démarrent muets.
- **filtre** : passe-bas 24 ou 12, passe-haut, passe-bande ou coupe-bande, avec coupure, résonance, drive, suivi de clavier et quantité d’enveloppe.
- les enveloppes d’**ampli**, de **filtre** et de **mod**. L’enveloppe de modulation peut être envoyée vers la hauteur, la hauteur de l’osc 2, la position dans la table d’ondes, la largeur d’impulsion ou la coupure.
- **LFO 1** et **LFO 2** : chacun a une forme, une vitesse (libre ou synchronisée), une cible (hauteur, coupure, amplitude, panoramique, position dans la table d’ondes ou largeur d’impulsion) et une quantité.
- **unisson** : jusqu’à 16 voix empilées par note, avec désaccord (en cents), mélange et étalement stéréo, pour des sons larges et épais.
- **voix** : nombre de voix, mode poly / mono / legato, glissé, amplitude du pitch bend et **dérive** (une légère instabilité façon analogique).

## synthé fm {#fm-synth}

Quatre opérateurs, de **A** à **D**, qui modulent la fréquence les uns des autres. Le résultat va des cloches douces aux sons métalliques agressifs.

- **algorithme** : la façon dont les opérateurs sont reliés (11 choix). Cliquez sur l’un des schémas d’algorithme dans la fenêtre pour le choisir. Les opérateurs qui atteignent la sortie sont entendus ; ceux qui les alimentent changent leur timbre. Par exemple, `D→C→B→A` est une seule pile où seul A est entendu, et `A+B+C+D` joue les quatre côte à côte.
- chaque opérateur a une **onde**, un **ratio** de fréquence (grossier et fin) ou une fréquence **fixe**, un **niveau**, sa propre enveloppe, une sensibilité à la **vélocité** et un **feedback** (il se module lui-même).
- seul l’opérateur A est audible au départ : les autres opérateurs démarrent au niveau le plus bas. Montez B, C ou D pour entendre l’effet FM.
- un **filtre** (désactivé par défaut) avec sa propre enveloppe, un **LFO** qui peut agir sur la hauteur, le filtre ou le volume, un **timbre**, et l’**anti-repliement** (suréchantillonnage 2×, activé par défaut, pour des notes aiguës plus propres).

## sampler {#sampler}

Joue un fichier audio.

- La première fois que vous ouvrez la fenêtre d’un sampler, un sélecteur d’échantillons s’ouvre : cherchez dans votre bibliothèque et choisissez un échantillon, ou faites-en glisser un depuis le [navigateur](/docs/sounds-and-plugins/browser) sur le sampler.
- **mode** : **one-shot** joue tout l’échantillon à sa propre hauteur sur chaque note et ignore le relâchement des notes (idéal pour les coups de batterie) ; **chromatique** le joue plus haut ou plus bas selon la touche, par rapport à la **note fondamentale**, et s’arrête quand vous relâchez la note.
- **début** et **fin** choisissent la partie de l’échantillon qui est jouée ; **inverser** le joue à l’envers.
- **boucle** (mode chromatique) : en avant ou en aller-retour entre le début et la fin de la boucle, avec un **fondu enchaîné** pour masquer le clic au point de bouclage.
- un **filtre** (passe-bas, passe-haut ou passe-bande), une enveloppe, une sensibilité à la vélocité et jusqu’à 32 voix.
- **plage de bend** : jusqu’où un pitch bend complet déplace l’échantillon (2 demi-tons par défaut), dans les deux modes.

### mode tranches {#slice-mode}

Le sélecteur à droite de l’échantillon, **classique** | **tranches**, choisit comment le sampler joue. En mode **classique**, chaque touche joue la plage entre le début et la fin. En mode **tranches**, l’échantillon est découpé en **tranches** et chaque touche en joue une : la **première touche** (C1 par défaut) joue la tranche 1, la touche suivante la tranche 2, et ainsi de suite. Les autres touches restent muettes. C’est ainsi qu’on joue les coups d’une boucle de batterie depuis un clavier ou un clip MIDI.

- Passer en **tranches** sur un échantillon sans tranches trouve aussitôt ses transitoires (le début de chaque coup). Les tranches remplacent la forme d’onde, chaque marqueur portant le nom de sa touche.
- **découpage auto** redécoupe l’échantillon : par **transitoires** (montez la **sensibilité** pour plus de tranches), sur une **grille rythmique** (de 1/16 à 1 mesure, au tempo du projet) ou en **parts égales**.
- **Double-cliquez** sur la forme d’onde pour ajouter une tranche, **faites glisser** un marqueur pour le déplacer, et **double-cliquez** sur un marqueur (ou sélectionnez-le et appuyez sur [[key:Delete]], ou faites un clic droit dessus) pour le retirer.
- **Appuyez** sur une tranche pour l’entendre : elle joue à travers la piste, comme une note.
- **découper en drum rack** remplace le sampler par un [drum rack](/docs/making-music/drum-rack#slicing-a-sample-to-pads) avec un pad par tranche, et ouvre sa fenêtre.

Chaque action de découpage est une seule étape d’annulation.

## multisampler {#multisampler}

Joue des échantillons différents selon les touches et les vélocités, comme un piano échantillonné.

- Sa fenêtre affiche une carte de **zones** : les touches de gauche à droite, la vélocité de bas en haut. Chaque zone est un échantillon avec sa plage de touches, sa plage de vélocité, sa note fondamentale, son accord, son gain, son panoramique et sa boucle.
- **Déposez plusieurs fichiers audio** de votre ordinateur sur une touche : ils deviennent des zones réparties sur le clavier. Les notes fondamentales sont lues dans les noms de fichiers, comme `C3` ou `60`. Un échantillon seul peut aussi venir du navigateur.
- Cliquez sur une zone pour la sélectionner et la modifier dans l’inspecteur de zone, où vous pouvez saisir des valeurs. Faites glisser le corps d’une zone pour la déplacer, ou ses bords pour changer sa plage de touches ou de vélocité. [[key:Delete]] supprime la zone sélectionnée.
- **round robin** : quand plusieurs zones couvrent la même note, **cycle** les joue à tour de rôle et **aléatoire** en choisit une, pour que les notes répétées ne sonnent pas identiques.
- Le reste est commun à toutes les zones : volume, panoramique, transposition, **plage de bend** (jusqu’où va un pitch bend complet, 2 demi-tons par défaut), une enveloppe d’amplitude, un filtre avec sa propre enveloppe, le glissé et jusqu’à 64 voix. Un instrument converti en échantillonneur suit donc toujours le couloir de pitch bend d’un clip.
- **legato** : une note jouée pendant qu’une autre est tenue ne repart pas de zéro : le son glisse vers la nouvelle hauteur, sans nouvelle attaque, et revient à la première note quand vous relâchez la seconde (avec le **glissé**, le glissement prend ce temps). Quand la nouvelle note utilise un autre échantillon, celui-ci prend le relais au même moment de la note. Pratique pour les leads et les basses, instruments convertis compris.

## instrument externe {#external-instrument}

Joue un synthé matériel depuis Ethereal [[desktop]] : les notes partent vers le synthé par un port MIDI, et son audio revient par les entrées de votre interface audio.

- Dans sa fenêtre, choisissez le port et le canal de **sortie MIDI**, et les **entrées audio** où le synthé est branché.
- **mesurer** chronomètre l’aller-retour d’Ethereal au synthé et retour, et règle la **latence** pour que le synthé reste en place avec le reste du morceau.
- **gain** règle le niveau de l’audio qui revient.
- Si vous ouvrez le projet sur un ordinateur sans ces ports, ils restent sélectionnés mais sont marqués comme absents, et l’appareil reste muet jusqu’à leur retour.
- Dans l’application web, le routage est affiché mais ne peut pas être modifié.

L’**effet audio externe** fait la même chose pour un effet matériel : voir [les effets](/docs/making-music/effects#external-audio-effect).

## et ensuite {#where-next}

- [Les effets](/docs/making-music/effects) : façonnez le son de vos instruments.
- [Le drum rack](/docs/making-music/drum-rack) : construisez un kit à partir d’échantillons.
- [Modulation et sidechain](/docs/making-music/modulation-and-sidechain) : faites bouger les paramètres tout seuls.
