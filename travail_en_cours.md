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
