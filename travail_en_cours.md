# SaiLine — travail en cours

## État du chantier

SaiLine est intégré nativement à PhotoCraft comme outil vectoriel dédié. Le modèle canonique
conserve les points logiques, leurs identifiants et les attributs propres aux points. Le cache de
rendu est dérivé et reconstruit après les modifications.

Le travail actuel porte sur l’édition interactive de tracés, la largeur variable par point et la
robustesse du rendu lorsque plusieurs tracés vivent sur le même calque.

## Fonctionnalités terminées

- Outil SaiLine distinct dans la barre d’outils, avec icône dédiée et options natives.
- Création par clics successifs, aperçu interpolé et validation par Entrée, Done ou clic droit.
- Annulation du brouillon par Échap, Cancel, changement d’outil ou changement de document.
- Retour arrière, Ctrl+Z et Ctrl+Y pendant la création du brouillon.
- Calque `LineworkLayer` persistant, plusieurs tracés indépendants par calque et identifiants stables.
- Sauvegarde/rechargement `.pcraft` des points, couleurs, largeurs et facteurs de largeur.
- Interpolation Bézier passant par les points, avec gestion des points proches ou superposés.
- Tangentes asymétriques selon la longueur des segments voisins : les grands arcs restent souples,
  les virages courts restent serrés, et les points confondus gardent un angle dur. Au retour d'un
  long segment vers un court, la poignée sortante s'oriente vers le court segment sans reprendre
  la longueur de la longue poignée entrante ;
  l'extrémité libre prolonge légèrement la courbure uniquement si le dernier segment est plus court.
- La force supplémentaire du grand segment décroît progressivement quand deux longueurs se
  rejoignent : déplacer un point juste au-delà de l'égalité ne fait plus sauter la courbe voisine.
- Validation finale sans conserver le curseur comme poignée : deux points validés restent droits.
- Sélection simple, Shift-clic additif/retrait, sélection rectangulaire temporaire avec Shift-glisser.
- Surbrillance au survol et marqueurs ronds pour les points SaiLine.
- Déplacement individuel et collectif avec Ctrl-glisser.
- Insertion sur un segment avec Ctrl-glisser et suppression avec Alt-clic.
- Historique coalescé : un glissement complet correspond à une seule étape undo/redo.
- Commande atomique `linework.movePoints` pour déplacer de nombreuses ancres en une transaction.
- Largeur variable par point (`width_factor`), commande `linework.setPointWidth` et contrôle Point width.
- Outil SaiLine Width séparé, avec icône dédiée affichée à côté du pointeur classique.
- Glissement horizontal SaiLine Width prévisualisé en direct, avec sensibilité fine et validation coalescée au relâchement.
- Clic simple SaiLine Width pour appliquer immédiatement la largeur affichée aux points successifs et harmoniser un tracé.
- Après un glissement, la valeur Width de la barre supérieure est synchronisée avec l’épaisseur obtenue.
- Clic droit SaiLine Width pour désélectionner le point ou les points actifs.
- Couleur par tracé, commande `linework.setPathColor` et action Path color ← foreground.
- Ajout de nouveaux tracés au calque Linework actif au lieu de créer systématiquement un calque.
- Recomposition du cache avec tous les tracés après déplacement, insertion, suppression, largeur ou couleur.
- Rasterisation accessible via les commandes natives de calques.

## Travail en cours

Le rendu de largeur variable est en cours d’affinage. L’objectif est une enveloppe offset continue
autour de la courbe centrale, avec un rayon interpolé entre les points. Plusieurs versions ont été
testées : contour polygonal, union de disques puis enveloppe offset échantillonnée. La version
actuelle utilise une enveloppe offset dérivée d’échantillons de la courbe et des jonctions
circulaires pour les facteurs supérieurs à 1.

Le déplacement collectif est maintenant atomique côté moteur. Une légère latence visuelle peut
encore apparaître pendant un glissement rapide, car le cache complet de tous les tracés du calque
est reconstruit à chaque mouvement. Aucun doublon de tracé ne doit être affiché pendant le geste.

## Points d’attention

- Ne pas revenir à un cache rendu pour un seul tracé : cela ferait disparaître les autres tracés
  visuellement tout en les laissant présents et manipulables dans le modèle.
- Toute modification géométrique ou de style doit reconstruire le cache de tous les tracés du
  calque concerné.
- Les facteurs de largeur extrêmes doivent rester stables : éviter auto-intersections, pointes,
  poches noires ou discontinuités entre sections.
- Les points superposés doivent conserver le comportement d’angle dur validé précédemment.
- L’interpolation doit encore être comparée visuellement à SAI sur plusieurs tracés réels ; le
  quadrillage fourni a servi à ajuster les grands arcs et les retours courts. Sur la comparaison
  violette du 17 octobre, le grand arc inférieur rejoint largement l'épaisseur du trait SAI ;
  la verticale gauche reste décalée d'environ 4 à 6 pixels au milieu. Éviter de régler ce seul
  exemple au détriment des autres courbes. Une seconde référence quadrillée montre un écart sur
  la montée depuis le premier point (bleu à droite du centre rouge) et sur le long segment final
  inférieur (bleu légèrement plus bas). La largeur rouge variable rend la ligne centrale
  approximative. Une troisième référence SAI à trait fin confirme que la correction d'extrémité
  des retours courts faussait une longue diagonale finale ; elle ne s'applique désormais plus à
  ce cas. Le dernier essai montre aussi une boucle vers l'extérieur sur un retour très court ; la
  poignée sortante a été réorientée sans augmenter sa longueur. Revoir visuellement ce tracé après
  compilation et continuer les comparaisons.
- La référence « 8 cases puis 7 ou 8 cases » a mis en évidence une discontinuité au passage des
  longueurs égales. Le calcul est désormais continu. Des essais systématiques ont révélé et corrigé
  deux autres sauts : l'activation de la poignée spéciale à l'extrémité et l'annulation de deux
  directions opposées dans un demi-tour. Les courbes restent à vérifier à la main dans l'application.
- Sur la comparaison des trois formes en L, SAI commence l'arc en montant et revient au dernier
  point depuis l'extérieur, tandis que SaiLine restait trop droit aux deux extrémités des cas 2
  et 3. Les tangentes libres prolongent maintenant légèrement le virage quand les segments sont
  de longueurs comparables, avec extinction progressive pour les cas courts, très déséquilibrés
  ou presque inversés. Vérifier visuellement les trois formes après recompilation.
- La pression réelle du stylet n’est pas encore reliée à `width_factor` ; la largeur est éditée
  manuellement.
- La courbure globale réglable, la transformation Ctrl+T de points, le copier/coller, la scission,
  le raccordement et le déplacement d’un tracé entier restent à implémenter.
- Aucun framework de plug-ins n’est à créer par anticipation.

## Vérifications récentes

Avec `CARGO_HOME=/mnt/SSD2/Codex/.tooling/cargo` :

- tests vectoriels SaiLine : OK ;
- tests d’intégration moteur Linework : 6 réussis ;
- tests UI SaiLine : OK ;
- `cargo check -p photocraft-engine -p photocraft-ui-egui --offline` : OK ;
- Clippy moteur, vectoriel et UI avec `-D warnings` : OK ;
- `cargo fmt --all` et `git diff --check` : OK.

## Commandes principales

- `linework.create`
- `linework.movePoint`
- `linework.movePoints`
- `linework.insertPoint`
- `linework.deletePoint`
- `linework.setPointWidth`
- `linework.setPathColor`
