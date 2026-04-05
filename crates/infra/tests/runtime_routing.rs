use infra::config::ReadConsistency;
use infra::runtime::{ReadRoute, Readiness, resolve_read_route};

#[test]
fn strong_requires_write_ready() {
    let readiness = Readiness {
        read_ready: true,
        write_ready: false,
    };

    let err = resolve_read_route(ReadConsistency::Strong, readiness).expect_err("must fail");
    assert!(err.to_string().contains("strong read"));
}

#[test]
fn prefer_strong_falls_back_to_local_when_write_not_ready() {
    let readiness = Readiness {
        read_ready: true,
        write_ready: false,
    };

    let route = resolve_read_route(ReadConsistency::PreferStrong, readiness).expect("route");
    assert_eq!(route, ReadRoute::Local);
}

#[test]
fn prefer_strong_uses_strong_when_write_is_ready() {
    let readiness = Readiness {
        read_ready: true,
        write_ready: true,
    };

    let route = resolve_read_route(ReadConsistency::PreferStrong, readiness).expect("route");
    assert_eq!(route, ReadRoute::Strong);
}
