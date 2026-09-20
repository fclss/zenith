use std::path::Path;

use serde_json::{Map, Value};

use crate::core::{CoreError, Node, Rule, RulePolicy};

/// The domain view of a subscription file: nodes and rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSubscription {
    pub nodes: Vec<Node>,
    pub rules: Vec<Rule>,
}

/// Read and parse a sing-box JSON subscription file.
pub fn parse_file(path: &Path) -> Result<ParsedSubscription, CoreError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| CoreError(format!("read {}: {}", path.display(), e)))?;
    parse_str(&contents)
}

/// Parse a sing-box JSON subscription payload.
pub fn parse_str(json: &str) -> Result<ParsedSubscription, CoreError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| CoreError(format!("invalid subscription JSON: {}", e)))?;
    let nodes = nodes_from_config(&value);
    if nodes.is_empty() {
        return Err(CoreError("subscription has no outbounds".to_string()));
    }
    let rules = rules_from_config(&value);
    Ok(ParsedSubscription { nodes, rules })
}

/// Every outbound's tag becomes a node. The injected `proxy` selector and
/// built-in helpers (direct/block) are excluded.
pub fn nodes_from_config(config: &Value) -> Vec<Node> {
    let Some(outbounds) = config.get("outbounds").and_then(|o| o.as_array()) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for outbound in outbounds {
        let tag = outbound.get("tag").and_then(|t| t.as_str());
        let typ = outbound.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if tag.is_none() {
            continue;
        }
        if typ == "selector"
            || typ == "urltest"
            || typ == "direct"
            || typ == "block"
            || typ == "dns"
        {
            continue;
        }
        nodes.push(Node {
            name: tag.unwrap().to_string(),
            delay: None,
        });
    }
    nodes
}

/// Route rules become rule-sets. A rule's predicate keys name it; its outbound
/// decides the policy (anything not `direct` is proxied through the tunnel).
pub fn rules_from_config(config: &Value) -> Vec<Rule> {
    let Some(rules) = config
        .get("route")
        .and_then(|r| r.get("rules"))
        .and_then(|r| r.as_array())
    else {
        return Vec::new();
    };
    let mut sets = Vec::new();
    for rule in rules {
        let outbound = rule.get("outbound").and_then(|o| o.as_str());
        let Some(outbound) = outbound else {
            continue;
        };
        let policy = if outbound == "direct" {
            RulePolicy::Direct
        } else {
            RulePolicy::Proxied
        };
        let name = rule_name(rule, outbound);
        sets.push(Rule { name, policy });
    }
    sets
}

fn rule_name(rule: &Value, outbound: &str) -> String {
    const PREDICATE_KEYS: &[&str] = &[
        "domain",
        "domain_suffix",
        "domain_keyword",
        "domain_regex",
        "geosite",
        "geoip",
        "ip_cidr",
        "port",
        "source_port",
        "network",
        "protocol",
        "process_name",
    ];
    for key in PREDICATE_KEYS {
        if let Some(values) = rule.get(key) {
            let parts: Vec<String> = match values {
                Value::Array(items) => items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .take(2)
                    .map(|s| format!("{key}:{s}"))
                    .collect(),
                Value::String(s) => vec![format!("{key}:{s}")],
                _ => Vec::new(),
            };
            if !parts.is_empty() {
                return parts.join(",");
            }
        }
    }
    outbound.to_string()
}

/// Tags of the outbounds that should go into the client-injected selector.
/// Wraps every plain outbound in a config that does not already have a
/// `proxy` selector.
pub fn node_tags(config: &Value) -> Vec<String> {
    nodes_from_config(config)
        .into_iter()
        .map(|n| n.name)
        .collect()
}

/// Whether the config already defines a selector (or urltest) named `proxy`.
pub fn has_proxy_group(config: &Value) -> bool {
    let Some(outbounds) = config.get("outbounds").and_then(|o| o.as_array()) else {
        return false;
    };
    outbounds.iter().any(|o| {
        let tag = o.get("tag").and_then(|t| t.as_str()).unwrap_or("");
        let typ = o.get("type").and_then(|t| t.as_str()).unwrap_or("");
        tag == "proxy" && (typ == "selector" || typ == "urltest")
    })
}

/// Build the final sing-box config the client runs: the subscription as-is,
/// plus the injected Clash API block, the mode's inbound (replacing whatever
/// the subscription shipped), a `proxy` selector wrapping the plain outbounds,
/// and a `direct` outbound if one is referenced. Android-only and legacy
/// config knobs that sing-box 1.12+ rejects on Linux are stripped (ADR-0002).
pub fn inject_control(
    config: &Value,
    mode: crate::core::Mode,
    port: u16,
    secret: &str,
) -> Result<Value, CoreError> {
    let mut root = config.clone();
    let root_map = root
        .as_object_mut()
        .ok_or_else(|| CoreError("subscription is not a JSON object".to_string()))?;

    root_map.insert(
        "experimental".to_string(),
        serde_json::json!({
            "clash_api": {
                "external_controller": format!("127.0.0.1:{port}"),
                "secret": secret,
                "default_mode": "rule",
            }
        }),
    );

    let inbound = match mode {
        crate::core::Mode::Tun => serde_json::json!({
            "type": "tun",
            "tag": "tun-in",
            "interface_name": "zenith",
            "address": ["172.28.0.1/30"],
            "auto_route": true,
            "strict_route": false,
            "stack": "gvisor",
            "iproute2_table_index": crate::core::singbox::TUN_ROUTE_TABLE,
            "iproute2_rule_index": crate::core::singbox::TUN_RULE_INDEX,
            "route_address": ["0.0.0.0/1", "128.0.0.0/1", "::/1", "8000::/1"],
            "route_exclude_address": ["192.168.0.0/16", "10.0.0.0/8", "fc00::/7"],
        }),
        crate::core::Mode::Proxy => serde_json::json!({
            "type": "mixed",
            "tag": "mixed-in",
            "listen": "127.0.0.1",
            "listen_port": crate::core::singbox::MIXED_PORT,
        }),
    };
    // The client owns the inbound: replace whatever the subscription shipped
    // (a provider's own tun/mixed inbound may need root, clash with our tag, or
    // carry legacy fields that sing-box 1.12+ refuses to decode).
    let inbounds = vec![inbound];
    root_map.insert("inbounds".to_string(), serde_json::json!(inbounds));
    root_map
        .entry("route".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let Some(route) = root_map.get_mut("route").and_then(|r| r.as_object_mut()) {
        route.remove("override_android_vpn");
    }

    // Fix DNS for TUN mode: set route.default_domain_resolver so sing-box
    // knows which DNS server to use when resolving domains in outbounds and
    // when mapping fakeip addresses back to real IPs. Prefer a `direct`-detoured
    // resolver: this path resolves the *proxy server's* own address, so routing
    // it back through the tunnel would be circular.
    if mode == crate::core::Mode::Tun {
        if let Some(dns) = root_map.get("dns") {
            let servers = dns.get("servers").and_then(|s| s.as_array());
            let is_plain_tagged = |s: &serde_json::Value| {
                let addr = s.get("address").and_then(|a| a.as_str()).unwrap_or("");
                let tag = s.get("tag").and_then(|t| t.as_str()).unwrap_or("");
                !addr.contains("fakeip") && !tag.is_empty()
            };
            let is_direct =
                |s: &serde_json::Value| s.get("detour").and_then(|d| d.as_str()) == Some("direct");
            let resolver_tag = servers
                .and_then(|servers| {
                    servers
                        .iter()
                        .find(|s| is_plain_tagged(s) && is_direct(s))
                        .or_else(|| servers.iter().find(|s| is_plain_tagged(s)))
                        .and_then(|s| s.get("tag").and_then(|t| t.as_str()))
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "local".to_string());

            if let Some(route) = root_map.get_mut("route").and_then(|r| r.as_object_mut()) {
                route.insert(
                    "default_domain_resolver".to_string(),
                    serde_json::json!(resolver_tag),
                );
            }
        }
    }

    let tags = node_tags(config);
    if !tags.is_empty() && !has_proxy_group(config) {
        let outbounds = root_map
            .entry("outbounds")
            .or_insert_with(|| serde_json::json!([]));
        if let Some(list) = outbounds.as_array_mut() {
            list.push(serde_json::json!({
                "type": "selector",
                "tag": "proxy",
                "outbounds": tags,
            }));
        }
    }
    if let Some(route) = root_map.get_mut("route").and_then(|r| r.as_object_mut()) {
        route.insert("final".to_string(), serde_json::json!("proxy"));
    }

    ensure_outbound(root_map, "direct", "direct");
    ensure_outbound(root_map, "block", "block");

    Ok(root)
}

fn ensure_outbound(root: &mut Map<String, Value>, tag: &str, typ: &str) {
    let outbounds = root
        .entry("outbounds")
        .or_insert_with(|| serde_json::json!([]));
    if let Some(list) = outbounds.as_array_mut() {
        if !list
            .iter()
            .any(|o| o.get("tag").and_then(|t| t.as_str()) == Some(tag))
        {
            list.push(serde_json::json!({ "type": typ, "tag": tag }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUB: &str = r#"{
        "dns": {},
        "outbounds": [
            {"type": "vless", "tag": "jp-1", "server": "1.2.3.4", "server_port": 443},
            {"type": "vless", "tag": "sg-2", "server": "5.6.7.8", "server_port": 443},
            {"type": "direct", "tag": "direct"}
        ],
        "route": {
            "rules": [
                {"action": "route", "outbound": "direct", "geoip": ["cn"]},
                {"action": "route", "outbound": "proxy", "geosite": ["netflix"]}
            ],
            "final": "proxy"
        }
    }"#;

    #[test]
    fn parses_outbounds_into_nodes() {
        let parsed = parse_str(SUB).unwrap();
        let names: Vec<String> = parsed.nodes.iter().map(|n| n.name.clone()).collect();
        assert_eq!(names, vec!["jp-1", "sg-2"]);
    }

    #[test]
    fn parses_rules_with_policies() {
        let parsed = parse_str(SUB).unwrap();
        assert_eq!(parsed.rules[0].name, "geoip:cn");
        assert_eq!(parsed.rules[0].policy, RulePolicy::Direct);
        assert_eq!(parsed.rules[1].name, "geosite:netflix");
        assert_eq!(parsed.rules[1].policy, RulePolicy::Proxied);
    }

    #[test]
    fn rejects_subscription_without_outbounds() {
        let err = parse_str(r#"{"dns":{}}"#).unwrap_err();
        assert!(err.0.contains("no outbounds"));
    }

    #[test]
    fn injects_selector_and_control_when_missing() {
        let base: Value = serde_json::from_str(SUB).unwrap();
        let injected = inject_control(&base, crate::core::Mode::Tun, 9090, "secret").unwrap();
        let tags: Vec<String> = injected["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|o| o["tag"].as_str())
            .map(|s| s.to_string())
            .collect();
        assert!(tags.contains(&"proxy".to_string()));
        assert!(tags.contains(&"direct".to_string()));
        assert!(tags.contains(&"block".to_string()));
        let sel = injected["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == "proxy")
            .unwrap();
        assert_eq!(sel["outbounds"], serde_json::json!(["jp-1", "sg-2"]));
        assert_eq!(injected["experimental"]["clash_api"]["secret"], "secret");
    }

    #[test]
    fn default_domain_resolver_prefers_direct_detour() {
        let base: Value = serde_json::from_str(
            r#"{
                "dns": {
                    "servers": [
                        {"tag": "cf-dns", "address": "tls://1.1.1.1"},
                        {"tag": "local", "address": "tcp://1.1.1.1", "detour": "direct"},
                        {"tag": "remote", "address": "fakeip"}
                    ]
                },
                "outbounds": [{"type": "vless", "tag": "a"}]
            }"#,
        )
        .unwrap();
        let injected = inject_control(&base, crate::core::Mode::Tun, 9090, "s").unwrap();
        assert_eq!(injected["route"]["default_domain_resolver"], "local");
    }

    #[test]
    fn does_not_duplicate_existing_proxy_selector() {
        let base: Value = serde_json::from_str(
            r#"{"outbounds":[
                {"type":"vless","tag":"a"},
                {"type":"selector","tag":"proxy","outbounds":["a"]}
            ]}"#,
        )
        .unwrap();
        let injected = inject_control(&base, crate::core::Mode::Proxy, 9090, "s").unwrap();
        let selectors: Vec<&Value> = injected["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["type"] == "selector" && o["tag"] == "proxy")
            .collect();
        assert_eq!(selectors.len(), 1);
    }

    #[test]
    fn client_owns_inbound_replacing_subscription_shipped_one() {
        let base: Value = serde_json::from_str(
            r#"{"inbounds":[
                {"type":"mixed","tag":"mixed-in","listen":"127.0.0.1","listen_port":2412,"set_system_proxy":true},
                {"type":"tun","tag":"tun-in","interface_name":"provider","inet4_address":"172.19.0.1/30"}
            ]}"#,
        )
        .unwrap();
        let injected = inject_control(&base, crate::core::Mode::Proxy, 9090, "s").unwrap();
        let inbounds = injected["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1, "provider inbounds must be dropped");
        assert_eq!(inbounds[0]["type"], "mixed");
        assert_eq!(inbounds[0]["listen_port"], crate::core::singbox::MIXED_PORT);
    }

    #[test]
    fn strips_android_only_route_knob() {
        let base: Value = serde_json::from_str(
            r#"{"route":{"override_android_vpn":true,"final":"direct"},"outbounds":[{"type":"direct","tag":"direct"}]}"#,
        )
        .unwrap();
        let injected = inject_control(&base, crate::core::Mode::Proxy, 9090, "s").unwrap();
        assert!(injected["route"].get("override_android_vpn").is_none());
    }
}
