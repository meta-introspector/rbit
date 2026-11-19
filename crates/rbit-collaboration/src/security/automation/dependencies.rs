//! Bounded npm lock inventory and deliberately resolver-free direct leaf updates.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_LOCK_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PACKAGES: usize = 1000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub location: String,
    pub name: String,
    pub version: String,
}
pub fn inventory(bytes: &[u8]) -> Result<Vec<Package>, &'static str> {
    let lock = document(bytes)?;
    let packages = lock["packages"].as_object().ok_or("The lockfile must contain a packages map.")?;
    if packages.len() > MAX_PACKAGES + 1 {
        return Err("The lockfile exceeds 1,000 packages.");
    }
    let mut result = Vec::new();
    for (location, value) in packages {
        if location.is_empty() {
            continue;
        }
        let name = location.rsplit_once("node_modules/").map(|(_, name)| name).ok_or("Workspace links are unsupported.")?;
        let version = value["version"].as_str().ok_or("Every package needs an exact version.")?;
        if !package_name(name)
            || location.len() > 1024
            || location.split('/').any(|part| part == ".." || part == ".")
            || value["link"] == true
            || !registry_url(value["resolved"].as_str().unwrap_or_default())
            || semver::Version::parse(version).is_err()
            || value.get("name").is_some_and(|n| n != name)
        {
            return Err("Only public npm registry packages with exact versions are supported; no aliases, links or private registries.");
        }
        result.push(Package { location: location.clone(), name: name.into(), version: version.into() });
    }
    Ok(result)
}
pub fn document(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() > MAX_LOCK_BYTES {
        return Err("The lockfile exceeds 2 MiB.");
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid package-lock JSON.")?;
    if value["lockfileVersion"] != 3 {
        return Err("Only npm package-lock version 3 is supported.");
    }
    Ok(value)
}
pub fn package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 214
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@/._-".contains(&b))
        && !name.contains("..")
        && if let Some(scoped) = name.strip_prefix('@') {
            scoped.split_once('/').is_some_and(|(scope, name)| !scope.is_empty() && !name.is_empty() && !name.contains('/'))
        } else {
            !name.contains(['/', '@']) && !name.starts_with(['.', '_'])
        }
}
pub fn registry_url(value: &str) -> bool {
    value.starts_with("https://registry.npmjs.org/")
        && value.len() <= 2048
        && !value.chars().any(char::is_control)
        && !value.contains(['?', '#', '\\'])
}
/// No package manager, installation or repository scripts run. Unsupported graphs require manual repair.
pub fn repair(manifest: &[u8], lock: &[u8], package: &Package, metadata: &Value) -> Result<(String, String), &'static str> {
    let inventory = inventory(lock)?;
    if inventory.iter().filter(|p| p.name == package.name).count() != 1
        || !inventory.contains(package)
        || package.location != format!("node_modules/{}", package.name)
    {
        return Err("Automatic repair supports one direct package occurrence only.");
    }
    if manifest.len() > MAX_LOCK_BYTES {
        return Err("The manifest exceeds 2 MiB.");
    }
    let mut manifest: Value = serde_json::from_slice(manifest).map_err(|_| "Invalid package manifest.")?;
    let mut lock = document(lock)?;
    let old = &lock["packages"][&package.location];
    for field in ["dependencies", "optionalDependencies", "peerDependencies", "bundleDependencies", "bundledDependencies", "workspaces"] {
        if old.get(field).is_some() || metadata.get(field).is_some() {
            return Err("This update requires dependency graph resolution; repair it manually.");
        }
    }
    if manifest.get("workspaces").is_some()
        || manifest.get("overrides").is_some()
        || metadata.get("deprecated").is_some()
        || old.get("hasInstallScript").is_some_and(|v| v == true)
        || metadata.get("hasInstallScript").is_some_and(|v| v == true)
        || ["preinstall", "install", "postinstall", "prepare"].iter().any(|key| metadata["scripts"].get(*key).is_some())
        || ["optionalDependencies", "peerDependencies"].iter().any(|field| manifest[*field].get(&package.name).is_some())
    {
        return Err("Workspace, overridden, deprecated or install-script packages require manual repair.");
    }
    for (location, value) in lock["packages"].as_object().ok_or("Missing packages.")? {
        if location.is_empty() {
            continue;
        }
        for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
            if value[field].get(&package.name).is_some() {
                return Err("Other packages depend on this version; repair it manually.");
            }
        }
    }
    let version = metadata["version"].as_str().ok_or("Missing replacement version.")?;
    let before = semver::Version::parse(&package.version).map_err(|_| "Invalid current version.")?;
    let after = semver::Version::parse(version).map_err(|_| "Invalid replacement version.")?;
    if after <= before
        || after.major != before.major
        || after.minor != before.minor
        || !after.pre.is_empty()
        || !after.build.is_empty()
        || metadata["name"] != package.name
        || !registry_url(metadata["dist"]["tarball"].as_str().unwrap_or_default())
    {
        return Err("Only a newer stable patch of the same public package can be proposed.");
    }
    let integrity = metadata["dist"]["integrity"].as_str().ok_or("Missing package integrity.")?;
    if !integrity.starts_with("sha512-")
        || integrity.len() != 95
        || !integrity.ends_with("==")
        || !integrity[7..93].bytes().all(|b| b.is_ascii_alphanumeric() || b"+/".contains(&b))
        || !b"AQgw".contains(&integrity.as_bytes()[92])
    {
        return Err("A bounded SHA-512 package integrity value is required.");
    }
    let sections: Vec<_> =
        ["dependencies", "devDependencies"].into_iter().filter(|section| manifest[*section].get(&package.name).is_some()).collect();
    if sections.len() != 1 {
        return Err("Only pinned direct dependencies or devDependencies can be repaired.");
    }
    let section = sections[0];
    if manifest[section][&package.name] != package.version || lock["packages"][""][section][&package.name] != package.version {
        return Err("The direct dependency must be pinned to the exact scanned version.");
    }
    manifest[section][&package.name] = version.into();
    lock["packages"][""][section][&package.name] = version.into();
    let entry = &mut lock["packages"][&package.location];
    entry["version"] = version.into();
    entry["resolved"] = metadata["dist"]["tarball"].clone();
    entry["integrity"] = integrity.into();
    // Version-specific metadata must come from the new registry record, never the old version.
    for field in ["engines", "os", "cpu", "libc", "license", "hasInstallScript"] {
        let object = entry.as_object_mut().ok_or("Invalid package entry.")?;
        object.remove(field);
        if let Some(value) = metadata.get(field) {
            object.insert(field.into(), value.clone());
        }
    }
    Ok((
        format!("{}\n", serde_json::to_string_pretty(&manifest).map_err(|_| "Cannot encode manifest.")?),
        format!("{}\n", serde_json::to_string_pretty(&lock).map_err(|_| "Cannot encode lockfile.")?),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_private_registries_and_workspace_links() {
        for value in
            [serde_json::json!({"version":"1.0.0","resolved":"http://localhost/a"}), serde_json::json!({"version":"1.0.0","link":true})]
        {
            let lock = serde_json::json!({"lockfileVersion":3,"packages":{"node_modules/demo":value}});
            assert!(inventory(&serde_json::to_vec(&lock).unwrap()).is_err());
        }
        assert!(!package_name("@scope/../secret"));
        assert!(package_name("@scope/package"));
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn repairs_exact_leaf_lock_consistently_and_refuses_graph_or_scripts() {
        let manifest = json!({"name":"demo-app", "dependencies":{"demo":"1.2.0"}});
        let lock = json!({"lockfileVersion":3,"packages":{"":{"dependencies":{"demo":"1.2.0"}},"node_modules/demo":{"version":"1.2.0","resolved":"https://registry.npmjs.org/demo/-/demo-1.2.0.tgz","integrity":"old"}}});
        let metadata = json!({"name":"demo","version":"1.2.1","dist":{"tarball":"https://registry.npmjs.org/demo/-/demo-1.2.1.tgz", "integrity":format!("sha512-{}==", "A".repeat(86))}});
        let (manifest, lock) = (serde_json::to_vec(&manifest).unwrap(), serde_json::to_vec(&lock).unwrap());
        let package = inventory(&lock).unwrap().remove(0);
        let (changed, changed_lock) = repair(&manifest, &lock, &package, &metadata).unwrap();
        let changed: Value = serde_json::from_str(&changed).unwrap();
        let changed_lock: Value = serde_json::from_str(&changed_lock).unwrap();
        assert_eq!(changed["dependencies"]["demo"], "1.2.1");
        assert_eq!(changed_lock["packages"][""]["dependencies"]["demo"], "1.2.1");
        assert_eq!(changed_lock["packages"]["node_modules/demo"]["version"], "1.2.1");
        assert_eq!(changed_lock["packages"]["node_modules/demo"]["integrity"], metadata["dist"]["integrity"]);
        for field in ["dependencies", "peerDependencies", "optionalDependencies"] {
            let mut unsupported = metadata.clone();
            unsupported[field] = json!({"new":"*"});
            assert!(repair(&manifest, &lock, &package, &unsupported).is_err());
        }
        let mut scripted = metadata;
        scripted["scripts"] = json!({"install":"touch unwanted"});
        assert!(repair(&manifest, &lock, &package, &scripted).is_err());
    }
}
