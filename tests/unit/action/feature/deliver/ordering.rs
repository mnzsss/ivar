use super::fixture::*;
use super::*;

#[test]
fn ordering_puts_a_repos_dependencies_before_it() {
    let mut repos = vec![
        delivery_repo("api", vec!["web"]),
        delivery_repo("web", vec![]),
        delivery_repo("cron", vec![]),
    ];

    order_by_dependencies(&mut repos);

    let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
    let web = order.iter().position(|name| *name == "web").unwrap();
    let api = order.iter().position(|name| *name == "api").unwrap();
    assert!(web < api, "a dependency must be pushed first: {order:?}");
    assert_eq!(order.len(), 3);
}

#[test]
fn ordering_preserves_name_order_between_unrelated_repos() {
    let mut repos = vec![
        delivery_repo("b", vec![]),
        delivery_repo("a", vec![]),
        delivery_repo("c", vec![]),
    ];

    order_by_dependencies(&mut repos);

    let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
    assert_eq!(order, vec!["b", "a", "c"], "no dependencies, no reordering");
}
