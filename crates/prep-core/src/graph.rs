use prep_manifest::{Lockfile, ModelError, PackageName};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    InvalidLock(String),
    MissingPackage { package: String, dependency: String },
    Cycle(Vec<String>),
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLock(message) => write!(formatter, "invalid lockfile graph: {message}"),
            Self::MissingPackage {
                package,
                dependency,
            } => write!(
                formatter,
                "package {package} depends on missing locked package {dependency}"
            ),
            Self::Cycle(cycle) => write!(formatter, "dependency cycle: {}", cycle.join(" -> ")),
        }
    }
}

impl Error for GraphError {}

impl From<ModelError> for GraphError {
    fn from(error: ModelError) -> Self {
        Self::InvalidLock(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyGraph {
    roots: Vec<PackageName>,
    dependencies: BTreeMap<PackageName, Vec<PackageName>>,
}

impl DependencyGraph {
    pub fn from_lockfile(lockfile: &Lockfile) -> Result<Self, GraphError> {
        lockfile.validate()?;

        let dependencies: BTreeMap<_, _> = lockfile
            .packages
            .iter()
            .map(|package| {
                let mut values = package.dependencies.clone();
                values.sort();
                (package.name.clone(), values)
            })
            .collect();

        let mut roots = lockfile.root.dependencies.clone();
        roots.sort();

        for dependency in &roots {
            if !dependencies.contains_key(dependency) {
                return Err(GraphError::MissingPackage {
                    package: "<root>".to_owned(),
                    dependency: dependency.to_string(),
                });
            }
        }

        for (package, package_dependencies) in &dependencies {
            for dependency in package_dependencies {
                if !dependencies.contains_key(dependency) {
                    return Err(GraphError::MissingPackage {
                        package: package.to_string(),
                        dependency: dependency.to_string(),
                    });
                }
            }
        }

        let graph = Self {
            roots,
            dependencies,
        };
        graph.build_order()?;
        Ok(graph)
    }

    pub fn build_order(&self) -> Result<Vec<PackageName>, GraphError> {
        let mut permanent = BTreeSet::new();
        let mut temporary = BTreeSet::new();
        let mut stack = Vec::new();
        let mut order = Vec::new();

        for root in &self.roots {
            self.visit(root, &mut permanent, &mut temporary, &mut stack, &mut order)?;
        }

        Ok(order)
    }

    fn visit(
        &self,
        package: &PackageName,
        permanent: &mut BTreeSet<PackageName>,
        temporary: &mut BTreeSet<PackageName>,
        stack: &mut Vec<PackageName>,
        order: &mut Vec<PackageName>,
    ) -> Result<(), GraphError> {
        if permanent.contains(package) {
            return Ok(());
        }
        if temporary.contains(package) {
            let start = stack.iter().position(|entry| entry == package).unwrap_or(0);
            let mut cycle: Vec<_> = stack[start..].iter().map(ToString::to_string).collect();
            cycle.push(package.to_string());
            return Err(GraphError::Cycle(cycle));
        }

        temporary.insert(package.clone());
        stack.push(package.clone());

        if let Some(dependencies) = self.dependencies.get(package) {
            for dependency in dependencies {
                self.visit(dependency, permanent, temporary, stack, order)?;
            }
        }

        stack.pop();
        temporary.remove(package);
        permanent.insert(package.clone());
        order.push(package.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prep_manifest::Lockfile;

    fn lock_with(edges: &[(&str, &[&str])], roots: &[&str]) -> Lockfile {
        let mut text = String::from(
            "schema = \"prep.lock/1\"\n\n[root]\nname = \"root\"\nversion = \"1\"\ndependencies = [",
        );
        for (index, root) in roots.iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            text.push_str(&format!("\"{root}\""));
        }
        text.push_str("]\n");

        for (name, dependencies) in edges {
            text.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"1\"\ndependencies = ["
            ));
            for (index, dependency) in dependencies.iter().enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                text.push_str(&format!("\"{dependency}\""));
            }
            text.push_str("]\n[package.source]\nkind = \"path\"\npath = \"fixture\"\n");
        }

        Lockfile::parse(&text).expect("fixture lock should parse")
    }

    #[test]
    fn transitive_build_order_is_dependency_first_and_deterministic() {
        let lock = lock_with(
            &[("appdep", &["zlib", "fmt"]), ("fmt", &[]), ("zlib", &[])],
            &["appdep"],
        );
        let graph = DependencyGraph::from_lockfile(&lock).expect("graph should build");
        let names: Vec<_> = graph
            .build_order()
            .expect("order should build")
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(names, ["fmt", "zlib", "appdep"]);
    }

    #[test]
    fn missing_transitive_dependency_fails() {
        let lock = lock_with(&[("a", &["missing"])], &["a"]);
        assert!(matches!(
            DependencyGraph::from_lockfile(&lock),
            Err(GraphError::MissingPackage { .. })
        ));
    }

    #[test]
    fn cycle_is_reported() {
        let lock = lock_with(&[("a", &["b"]), ("b", &["a"])], &["a"]);
        assert!(matches!(
            DependencyGraph::from_lockfile(&lock),
            Err(GraphError::Cycle(_))
        ));
    }
}
