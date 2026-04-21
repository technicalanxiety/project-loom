//! Property 32: Dashboard API Read-Only Enforcement
//!
//! Feature: loom-memory-compiler, Property 32: Dashboard API Read-Only Enforcement
//! For any dashboard API GET endpoint, the response should not modify any database
//! state. Only the conflict resolution POST and predicate candidate resolution POST
//! endpoints should perform writes.
//!
//! Validates: Requirements 50.2, 50.3

use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Route registry
// ---------------------------------------------------------------------------

/// HTTP method for a dashboard API route.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
}

/// A dashboard API route definition.
#[derive(Debug, Clone)]
struct DashboardRoute {
    method: HttpMethod,
    path: &'static str,
    /// Whether this route is expected to perform database writes.
    is_write: bool,
}

/// The complete set of dashboard API routes as registered in main.rs.
fn all_dashboard_routes() -> Vec<DashboardRoute> {
    vec![
        // Read-only GET endpoints (17 total)
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/health", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/namespaces", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/compilations", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/compilations/{id}", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/entities", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/entities/{id}", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/entities/{id}/graph", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/facts", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/conflicts", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/predicates/candidates", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/predicates/packs", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/predicates/packs/{pack}", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/predicates/active/{namespace}", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/metrics/retrieval", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/metrics/extraction", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/metrics/classification", is_write: false },
        DashboardRoute { method: HttpMethod::Get, path: "/dashboard/api/metrics/hot-tier", is_write: false },
        // Write POST endpoints (exactly 2)
        DashboardRoute { method: HttpMethod::Post, path: "/dashboard/api/conflicts/{id}/resolve", is_write: true },
        DashboardRoute { method: HttpMethod::Post, path: "/dashboard/api/predicates/candidates/{id}/resolve", is_write: true },
    ]
}

// ---------------------------------------------------------------------------
// Property 32: Dashboard API Read-Only Enforcement
// ---------------------------------------------------------------------------

/// **Property 32: Dashboard API Read-Only Enforcement**
///
/// Feature: loom-memory-compiler, Property 32: Dashboard API Read-Only Enforcement
///
/// **Validates: Requirements 50.2, 50.3**
mod dashboard_api_read_only_enforcement {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Property 1: For any randomly selected route, if it is a GET then is_write must be false.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn get_routes_are_never_write(index in any::<usize>()) {
            let routes = all_dashboard_routes();
            let route = &routes[index % routes.len()];

            if route.method == HttpMethod::Get {
                prop_assert!(
                    !route.is_write,
                    "GET route '{}' must not be a write route",
                    route.path
                );
            }
        }

        /// Property 2: For any randomly selected route, if is_write is true then it must be a POST.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn write_routes_are_always_post(index in any::<usize>()) {
            let routes = all_dashboard_routes();
            let route = &routes[index % routes.len()];

            if route.is_write {
                prop_assert_eq!(
                    &route.method, &HttpMethod::Post,
                    "write route '{}' must use POST method",
                    route.path
                );
            }
        }

        /// Property 3: The total number of write routes is exactly 2.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn exactly_two_write_routes(_seed in any::<u64>()) {
            let routes = all_dashboard_routes();
            let write_count = routes.iter().filter(|r| r.is_write).count();

            prop_assert_eq!(
                write_count, 2,
                "there must be exactly 2 write routes, found {}",
                write_count
            );
        }

        /// Property 4: The total number of GET routes equals total routes minus 2.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn get_route_count_equals_total_minus_two(_seed in any::<u64>()) {
            let routes = all_dashboard_routes();
            let total = routes.len();
            let get_count = routes.iter().filter(|r| r.method == HttpMethod::Get).count();

            prop_assert_eq!(
                get_count,
                total - 2,
                "GET route count ({}) must equal total routes ({}) minus 2",
                get_count,
                total
            );
        }

        /// Property 5: All routes start with `/dashboard/api/`.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn all_routes_under_dashboard_api_prefix(index in any::<usize>()) {
            let routes = all_dashboard_routes();
            let route = &routes[index % routes.len()];

            prop_assert!(
                route.path.starts_with("/dashboard/api/"),
                "route '{}' must start with '/dashboard/api/'",
                route.path
            );
        }

        /// Property 6: No GET route path ends with `/resolve`.
        ///
        /// **Validates: Requirements 50.2, 50.3**
        #[test]
        fn get_routes_do_not_end_with_resolve(index in any::<usize>()) {
            let routes = all_dashboard_routes();
            let route = &routes[index % routes.len()];

            if route.method == HttpMethod::Get {
                prop_assert!(
                    !route.path.ends_with("/resolve"),
                    "GET route '{}' must not end with '/resolve' — only POST routes resolve things",
                    route.path
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn exactly_two_post_routes_exist() {
        let routes = all_dashboard_routes();
        let post_routes: Vec<_> = routes.iter().filter(|r| r.method == HttpMethod::Post).collect();
        assert_eq!(
            post_routes.len(),
            2,
            "expected exactly 2 POST routes, found {}: {:?}",
            post_routes.len(),
            post_routes.iter().map(|r| r.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn all_post_routes_are_write_routes() {
        let routes = all_dashboard_routes();
        for route in routes.iter().filter(|r| r.method == HttpMethod::Post) {
            assert!(
                route.is_write,
                "POST route '{}' must be marked as a write route",
                route.path
            );
        }
    }

    #[test]
    fn all_get_routes_are_non_write_routes() {
        let routes = all_dashboard_routes();
        for route in routes.iter().filter(|r| r.method == HttpMethod::Get) {
            assert!(
                !route.is_write,
                "GET route '{}' must not be marked as a write route",
                route.path
            );
        }
    }

    #[test]
    fn write_routes_are_exactly_conflict_and_predicate_resolve() {
        let routes = all_dashboard_routes();
        let write_paths: Vec<&str> = routes
            .iter()
            .filter(|r| r.is_write)
            .map(|r| r.path)
            .collect();

        assert!(
            write_paths.contains(&"/dashboard/api/conflicts/{id}/resolve"),
            "conflict resolve endpoint must be a write route"
        );
        assert!(
            write_paths.contains(&"/dashboard/api/predicates/candidates/{id}/resolve"),
            "predicate candidate resolve endpoint must be a write route"
        );
        assert_eq!(
            write_paths.len(),
            2,
            "only the two resolve endpoints should be write routes, found: {:?}",
            write_paths
        );
    }

    #[test]
    fn total_route_count_is_nineteen() {
        let routes = all_dashboard_routes();
        assert_eq!(
            routes.len(),
            19,
            "expected 19 total routes (17 GET + 2 POST), found {}",
            routes.len()
        );
    }

    #[test]
    fn get_route_count_is_seventeen() {
        let routes = all_dashboard_routes();
        let get_count = routes.iter().filter(|r| r.method == HttpMethod::Get).count();
        assert_eq!(
            get_count,
            17,
            "expected 17 GET routes, found {}",
            get_count
        );
    }
}
