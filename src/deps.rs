use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::SpreadsheetError;

/// Build evaluation layers via Kahn's algorithm.
///
/// `dependencies[cell]` = numeric cells that `cell` depends on.
/// Returns layers in evaluation order, or `CircularReference` if a cycle exists.
pub(crate) fn build_layers(
    cells: &HashSet<String>,
    dependencies: &HashMap<String, HashSet<String>>,
) -> Result<Vec<Vec<String>>, SpreadsheetError> {
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

    for cell in cells {
        let deps = dependencies
            .get(cell)
            .map(|set| set.iter().filter(|d| cells.contains(*d)).count())
            .unwrap_or(0);
        indegree.insert(cell.clone(), deps);

        if let Some(deps) = dependencies.get(cell) {
            for dep in deps {
                if cells.contains(dep) {
                    dependents.entry(dep.clone()).or_default().push(cell.clone());
                }
            }
        }
    }

    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(name, _)| name.clone())
        .collect();

    let mut layers = Vec::new();
    let mut processed = 0usize;

    while !queue.is_empty() {
        let layer: Vec<String> = queue.drain(..).collect();
        processed += layer.len();

        let mut next = Vec::new();
        for cell in &layer {
            if let Some(children) = dependents.get(cell) {
                for child in children {
                    if let Some(deg) = indegree.get_mut(child) {
                        *deg -= 1;
                        if *deg == 0 {
                            next.push(child.clone());
                        }
                    }
                }
            }
        }

        layers.push(layer);
        queue.extend(next);
    }

    if processed != cells.len() {
        let cyclic = indegree
            .into_iter()
            .find(|(_, deg)| *deg > 0)
            .map(|(name, _)| name)
            .unwrap_or_else(|| "unknown".to_string());
        return Err(SpreadsheetError::CircularReference(cyclic));
    }

    Ok(layers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::analyze_formula_refs;

    #[test]
    fn extracts_cell_refs_but_not_functions_or_strings() {
        let refs = analyze_formula_refs("=SUM(A1, B2) + IF(C1 > 0, D1, \"A9\")").unwrap();
        assert!(refs.cells.contains("A1"));
        assert!(refs.cells.contains("B2"));
        assert!(refs.cells.contains("C1"));
        assert!(refs.cells.contains("D1"));
        assert!(!refs.cells.contains("SUM"));
        assert!(!refs.cells.contains("IF"));
        assert!(!refs.cells.contains("A9"));
    }

    #[test]
    fn extract_expands_ranges() {
        let refs = analyze_formula_refs("=SUM(A1:A3)").unwrap();
        assert_eq!(refs.cells.len(), 3);
        assert!(refs.cells.contains("A2"));
        assert_eq!(refs.work, 3);
    }

    #[test]
    fn builds_independent_layer_then_dependents() {
        let cells: HashSet<_> = ["A1", "A2", "B1"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut dependencies = HashMap::new();
        dependencies.insert("A1".to_string(), HashSet::new());
        dependencies.insert("A2".to_string(), HashSet::new());
        dependencies.insert(
            "B1".to_string(),
            ["A1", "A2"].into_iter().map(str::to_string).collect(),
        );

        let layers = build_layers(&cells, &dependencies).unwrap();
        assert_eq!(layers.len(), 2);
        let layer0: HashSet<_> = layers[0].iter().cloned().collect();
        assert_eq!(layer0, HashSet::from(["A1".to_string(), "A2".to_string()]));
        assert_eq!(layers[1], vec!["B1".to_string()]);
    }

    #[test]
    fn detects_cycle() {
        let cells: HashSet<_> = ["A1", "B1"].into_iter().map(str::to_string).collect();
        let mut dependencies = HashMap::new();
        dependencies.insert("A1".to_string(), HashSet::from(["B1".to_string()]));
        dependencies.insert("B1".to_string(), HashSet::from(["A1".to_string()]));

        assert!(matches!(
            build_layers(&cells, &dependencies),
            Err(SpreadsheetError::CircularReference(_))
        ));
    }
}
