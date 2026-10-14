// SPDX-License-Identifier: Apache-2.0
//
// Programmatic API-parity check.
//
// Traccar publishes its HTTP contract as an OpenAPI 3.1 document. This test is
// the guard rail for "API-compatible reimplementation": it parses the vendored
// contract (`docs/openapi.yaml`) and requires that every path/method is either
// implemented by the core router (`traccar_core::api::routes`) or explicitly
// listed as deferred with a reason (`traccar_core::api::deferred_routes`).
//
// It also checks the reverse direction, so the implementation cannot quietly
// grow routes that the contract does not define. Either drift direction fails
// the build.
//
// The OpenAPI document is read with a tiny line scanner rather than a YAML
// dependency: the two-line shape it needs (`  /path:` then `    method:`) is
// stable across Traccar releases and keeps the dependency graph small.

use std::collections::BTreeSet;

use traccar_core::api::{deferred_routes, routes};

/// Collapse `{param}` segments to `{}` and drop the `/api` server prefix so the
/// contract and the router table can be compared structurally.
fn normalize(path: &str) -> String {
    let p = path.strip_prefix("/api").unwrap_or(path);
    let p = p.trim_end_matches('/');
    let segs: Vec<String> = p
        .split('/')
        .filter(|s| !s.is_empty())
        .map(normalize_segment)
        .collect();
    format!("/{}", segs.join("/"))
}

fn normalize_segment(seg: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for ch in seg.chars() {
        match ch {
            '{' => {
                in_param = true;
                out.push('{');
            }
            '}' => {
                in_param = false;
                out.push('}');
            }
            _ if in_param => {}
            _ => out.push(ch),
        }
    }
    out
}

/// `(METHOD, normalized-path)` pairs declared in the OpenAPI document.
fn contract_routes() -> Vec<(String, String)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/openapi.yaml");
    let text = std::fs::read_to_string(path).expect("docs/openapi.yaml must be vendored");
    let lines: Vec<&str> = text.lines().collect();

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // A path entry is exactly two-space indented, starts with '/', ends ':'.
        if let Some(rest) = line.strip_prefix("  ") {
            if rest.starts_with('/') && rest.ends_with(':') && !rest.starts_with("//") {
                let raw = rest.trim_end_matches(':');
                let mut j = i + 1;
                // Methods are four-space indented and terminate at the next
                // path (two-space indent) or a top-level key.
                while j < lines.len() {
                    let l = lines[j];
                    if l.starts_with("  ") && !l.starts_with("    ") {
                        break;
                    }
                    if !l.starts_with(' ') && !l.is_empty() {
                        break;
                    }
                    if let Some(m) = l.strip_prefix("    ") {
                        if let Some(method) = m.strip_suffix(':') {
                            let up = method.to_ascii_uppercase();
                            if matches!(up.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH") {
                                out.push((up, normalize(raw)));
                            }
                        }
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[test]
fn every_contract_route_is_implemented_or_deferred() {
    let contract: BTreeSet<(String, String)> = contract_routes().into_iter().collect();
    assert!(!contract.is_empty(), "failed to parse any routes from docs/openapi.yaml");

    let mut implemented: BTreeSet<(String, String)> = BTreeSet::new();
    for (m, p) in routes() {
        assert!(
            implemented.insert((m.into(), normalize(p))),
            "duplicate entry in routes(): {m} {p}"
        );
    }

    let mut deferred: BTreeSet<(String, String)> = BTreeSet::new();
    for (m, p, reason) in deferred_routes() {
        assert!(!reason.trim().is_empty(), "deferred route {m} {p} needs a reason");
        assert!(
            deferred.insert((m.into(), normalize(p))),
            "duplicate entry in deferred_routes(): {m} {p}"
        );
    }

    // Nothing may be both implemented and deferred.
    let both: Vec<_> = implemented.intersection(&deferred).collect();
    assert!(both.is_empty(), "routes listed as both implemented and deferred: {both:?}");

    let covered: BTreeSet<(String, String)> = implemented.union(&deferred).cloned().collect();
    let missing: Vec<_> = contract.difference(&covered).collect();
    assert!(
        missing.is_empty(),
        "\n{} Traccar OpenAPI route(s) are neither implemented nor deferred:\n{}",
        missing.len(),
        missing.iter().map(|(m, p)| format!("  {m} {p}")).collect::<Vec<_>>().join("\n")
    );

    let extra: Vec<_> = implemented.difference(&contract).collect();
    assert!(
        extra.is_empty(),
        "\n{} implemented route(s) are not in the Traccar contract:\n{}",
        extra.len(),
        extra.iter().map(|(m, p)| format!("  {m} {p}")).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn contract_covers_all_expected_endpoints() {
    // A canary so a botched parse (e.g. an OpenAPI reformat) cannot make the
    // parity test vacuously pass.
    let contract: BTreeSet<(String, String)> = contract_routes().into_iter().collect();
    assert!(contract.len() >= 100, "expected >=100 contract operations, parsed {}", contract.len());
    for probe in [
        ("GET", "/devices"),
        ("POST", "/session"),
        ("GET", "/reports/summary"),
        ("GET", "/stream/{}/{}/live.m3u8"),
    ] {
        assert!(
            contract.contains(&(probe.0.to_string(), probe.1.to_string())),
            "contract probe missing: {} {}",
            probe.0,
            probe.1
        );
    }
}
