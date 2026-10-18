# SaiLine — travail en cours

## État du chantier

SaiLine est intégré nativement à PhotoCraft comme outil vectoriel dédié. Le modèle canonique
conserve les points logiques, leurs identifiants et les attributs propres aux points. Le cache de
rendu est dérivé et reconstruit après les modifications.

Le travail actuel porte sur l’édition interactive de tracés, la largeur variable par point et la
robustesse du rendu lorsque plusieurs tracés vivent sur le même calque.

## Fonctionnalités terminées

- Outil SaiCurve distinct dans la barre d’outils, avec icône dédiée et options natives.
- Création par clics successifs, aperçu interpolé avec extrémités rondes pendant le tracé, puis validation par Entrée, Done ou clic droit.
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
- Largeur de base indépendante (0,7–30 px) et pression propre à chaque point (`width_factor`, 100 % par défaut).
- Outil séparé **SaiPressure**, avec compatibilité de lecture pour l'ancien nom `SaiLineWidth`.
- SaiCurve et SaiPressure partagent Ctrl-clic pour insérer un point et Ctrl-glisser pour déplacer un ou plusieurs points.
- Dans SaiPressure, le clic simple ne modifie pas la pression ; le glisser horizontal ajuste de 0 à 200 %, actualise l’aperçu à 30 Hz maximum pour limiter les recalculs et enregistre la dernière valeur en une étape d’historique au relâchement.
- Clic droit dans SaiPressure désélectionne le point ou les points actifs.
- Couleur par tracé, commande `linework.setPathColor` et action Path color ← foreground.
- Ajout de nouveaux tracés au calque Linework actif au lieu de créer systématiquement un calque.
- Recomposition du cache avec tous les tracés après déplacement, insertion, suppression, largeur ou couleur.
- Rasterisation accessible via les commandes natives de calques.

## Travail en cours

La géométrie par défaut est désormais le modèle **SAI simplifié** validé (α=1/3, β=1/2,
tension −0,2, tangentes nulles aux extrémités), appelé directement depuis le banc expérimental.
Le modèle optimisé reste disponible dans les expériences ; le sélecteur de modèles a été retiré
de l'interface finale.

La recherche de SaiPressure est conservée dans `experiments/sailine_width/`. Les six cas de la
capture sont extraits, les largeurs sont mesurées selon les normales de la courbe, et la planche
de diagnostic ainsi que le classement sont reproductibles. L'interpolation linéaire par longueur
d'arc obtient 1,302 px d'erreur moyenne équilibrée contre 1,373 px pour le paramètre Bézier ;
0,071 px est inférieur à l'incertitude estimée (±1,5 px), donc ce résultat ne départage pas ces
deux lois. Le rendu de contour n'est pas encore confirmé comme équivalent pixel à pixel à Sai.

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
- La géométrie SAI simplifiée est maintenant le modèle par défaut. L'interpolation de pression
  linéaire en paramètre Bézier reste provisoire : la longueur d'arc score légèrement mieux mais
  l'écart est sous l'incertitude des mesures. Refaire une comparaison interactive avant de choisir.
- Le rendu du contour à pression variable doit encore être évalué en courbes serrées et zones de
  recouvrement. Les six captures et leurs zones mesurées sont documentées dans le banc.
- Sur la comparaison violette du 17 octobre, le grand arc inférieur rejoint largement l'épaisseur du trait SAI ;
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
- La pression physique du stylet n’est pas reliée à `width_factor` ; SaiPressure édite la valeur
  du point par glissement horizontal.
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

## Mise à jour — SaiPressure et interpolation par défaut

Le moteur de production utilise maintenant SaiSimplified par défaut sans modifier les points
historiques. L'ancien état outil `SaiLineWidth` est encore chargé par alias, tandis que l'interface
et les nouveaux états nomment le second outil SaiPressure. Les deux outils partagent Ctrl-clic et
Ctrl-glisser. SaiPressure ne change plus la pression au clic ; un glisser horizontal affiche et
valide 0–200 % au relâchement. Le déplacement et la pression gardent leur propre geste et une
transaction undo par glisser.

Le banc `experiments/sailine_width` contient `dataset.json`, la capture source, `diagnostic.png`,
`comparison_board.png`, `results.json`, le code Rust et les scripts Python. Les six cas ont été
scorés. La loi linéaire en longueur d'arc mène à 1,302 px, contre 1,373 px pour le paramètre Bézier ;
la différence est plus faible que la marge de mesure, donc l'interpolation de pression actuelle
reste à valider visuellement. L'extraction marque zéro mesure ambiguë, mais les recouvrements et
l'antialiasing demeurent une limite. Les tests géométriques, moteur et UI passent respectivement
76/77 (un test ignoré), 9/9 et 11/11 ; la compilation complète PhotoCraft passe.


## Retours récents — SaiCurve et performances SaiPressure

L'ancien nom d'outil SaiLine a été remplacé par SaiCurve dans l'interface et l'état Rust ; le
chargement accepte encore `SaiLine` pour les états sauvegardés. Pendant la création, le trait
prévisualisé reçoit maintenant des caps circulaires aux deux extrémités, comme le tracé validé.

Le changement de pression conserve la dernière valeur du curseur à chaque événement, mais ne
reconstruit plus le cache à chaque mouvement : l'aperçu moteur est plafonné à environ 30 images
par seconde, et la valeur exacte du relâchement est toujours calculée puis enregistrée. Le test UI
vérifie que les mises à jour rapides sont sautées et que le relâchement valide bien la dernière
valeur.
