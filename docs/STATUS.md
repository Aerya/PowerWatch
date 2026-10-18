# Évolutions tarifaires — 18 octobre 2026

La page `/tariffs` remplace les réglages tarifaires flottants de PowerWatch et du Hub. Elle propose un formulaire de contrat, un historique éditable par période, l’affectation exclusive des machines aux sites côté Hub et les outils d’import/export. Le catalogue préremplit le formulaire. Les changements de contrat et les nouvelles dates de prix conservent les périodes précédentes.

Les graphiques présentent le coût cumulé des kWh consommés depuis le début de la période, hors abonnement, sur un axe en euros. Le Hub utilise ses machines activées et incluses dans le total habituel et les contrats de leurs sites. Les périodes sans données ou sans prix interrompent la courbe ; les cumuls partiels et les estimations restent signalés.

Validation locale : 325 tests Rust et 48 tests JavaScript réussis. Les parcours navigateur ont été vérifiés sur deux services natifs isolés : sauvegarde et rechargement, conservation des grilles, copie et affectation des machines, bascule des périodes des graphiques et affichage à 1440 et 375 pixels. L’accès protégé, l’enregistrement avec protection CSRF et la courbe ont aussi été vérifiés avec l’authentification PowerWatch activée.

Les corrections antérieures du jeton administrateur du Hub et de la conservation des formulaires sont incluses. Les données de production n’ont pas été utilisées ni modifiées. La publication GitHub et le déploiement restent à effectuer.
