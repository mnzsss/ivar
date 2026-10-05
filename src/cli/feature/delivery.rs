use crate::action::feature::deliver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureDeliverArgs {
    /// The feature to deliver.
    pub feature: Option<String>,
    /// Print the delivery preview and push nothing.
    pub preview: bool,
    /// Land feature branches into default branches (fast-forward only), then
    /// push each default branch to its remote.
    pub land: bool,
    /// The fingerprint from the preview the human approved; required to apply.
    pub fingerprint: Option<String>,
    /// Global metadata.
    pub global_metadata: deliver::PullRequestMetadata,
    /// Repository-scoped overrides.
    pub repo_overrides: Vec<deliver::RepoMetadataOverride>,
    /// Promoted repositories to restrict delivery to.
    pub only: Vec<String>,
}

impl clap::Args for FeatureDeliverArgs {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        cmd.arg(
            clap::Arg::new("feature")
                .help("The feature to deliver.")
                .required(false)
                .index(1),
        )
        .arg(
            clap::Arg::new("preview")
                .long("preview")
                .help("Print the delivery preview and push nothing.")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("land")
                .long("land")
                .help("Land feature branches into default branches (fast-forward only), then push each default branch to its remote.")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("fingerprint")
                .long("fingerprint")
                .help("The fingerprint from the preview the human approved; required to apply. It covers `--name`, `--body`, `--draft` and `--only`, so apply with the same values the preview used. Apply recomputes the preview and refuses when the fingerprint differs — the state has drifted since the preview.")
                .value_name("FINGERPRINT"),
        )
        .arg(
            clap::Arg::new("name")
                .long("name")
                .help("Pull request title. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply.")
                .value_name("TITLE")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("body")
                .long("body")
                .help("Pull request body text, or a path to a `.md` / `.txt` file — either `./relative` or absolute. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply.")
                .value_name("BODY")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("repo")
                .long("repo")
                .help("Scope following `--name`, `--body`, and `--draft` flags to this promoted repository. Does not select which repositories are delivered; use `--only` for that.")
                .value_name("REPO")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("only")
                .long("only")
                .help("Deliver only this promoted repository; repeat to select several. Without it every promoted repository is delivered. Part of the delivery fingerprint: pass the same values to the preview and the apply.")
                .value_name("REPO")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("draft")
                .long("draft")
                .help("Create or convert a pull request to a draft. If placed before any `--repo`, applies globally to all repos; if placed after a `--repo`, applies only to that repo. Incompatible with `--land`.")
                // The flag must be repeatable AND keep one parser index per
                // occurrence, because position decides global vs repository
                // scope. `SetTrue` rejects the second occurrence; `Count`
                // collapses every occurrence onto a single index. `Append`
                // taking no value records an index and `true` per occurrence.
                .num_args(0)
                .value_parser(clap::value_parser!(bool))
                .default_missing_value("true")
                .action(clap::ArgAction::Append),
        )
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

impl clap::FromArgMatches for FeatureDeliverArgs {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        let feature = matches.get_one::<String>("feature").cloned();
        let preview = matches.get_flag("preview");
        let land = matches.get_flag("land");
        let fingerprint = matches.get_one::<String>("fingerprint").cloned();

        let only = matches
            .get_many::<String>("only")
            .map(|values| values.cloned().collect())
            .unwrap_or_default();

        let (global_metadata, repo_overrides) = Self::parse_metadata(matches)?;

        Ok(Self {
            feature,
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
            only,
        })
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

fn set_delivery_metadata<T>(
    slot: &mut Option<T>,
    value: T,
    option: &str,
    repo: Option<&str>,
) -> Result<(), clap::Error> {
    if slot.is_some() {
        let scope = repo.map_or_else(String::new, |repo| format!(" in repository group `{repo}`"));
        return Err(clap::Error::raw(
            clap::error::ErrorKind::ArgumentConflict,
            format!("duplicate `--{option}`{scope}\n"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

impl FeatureDeliverArgs {
    /// Reconstruct global metadata and ordered per-repo overrides from raw command line arguments.
    pub fn parse_metadata(
        matches: &clap::ArgMatches,
    ) -> Result<
        (
            deliver::PullRequestMetadata,
            Vec<deliver::RepoMetadataOverride>,
        ),
        clap::Error,
    > {
        enum DeliverOption {
            Name(String),
            Body(String),
            Repo(String),
            Draft,
        }

        let mut occurrences: Vec<(usize, DeliverOption)> = Vec::new();

        if let Some(indices) = matches.indices_of("name") {
            let values: Vec<&String> = matches
                .get_many::<String>("name")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Name(val.clone())));
            }
        }
        if let Some(indices) = matches.indices_of("body") {
            let values: Vec<&String> = matches
                .get_many::<String>("body")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Body(val.clone())));
            }
        }
        if let Some(indices) = matches.indices_of("repo") {
            let values: Vec<&String> = matches
                .get_many::<String>("repo")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Repo(val.clone())));
            }
        }
        // No `get_flag` guard: with `Append` the flag is not a bool-typed
        // value, and `indices_of` already returns `None` when it is absent.
        if let Some(indices) = matches.indices_of("draft") {
            for idx in indices {
                occurrences.push((idx, DeliverOption::Draft));
            }
        }

        occurrences.sort_by_key(|(idx, _)| *idx);

        let mut global_metadata = deliver::PullRequestMetadata::default();
        let mut repo_overrides: Vec<deliver::RepoMetadataOverride> = Vec::new();
        let mut current_repo: Option<(String, deliver::PullRequestMetadata)> = None;

        for (_, opt) in occurrences {
            match opt {
                DeliverOption::Repo(repo_name) => {
                    if let Some((r, meta)) = current_repo.take() {
                        repo_overrides.push(deliver::RepoMetadataOverride {
                            repo: r,
                            metadata: meta,
                        });
                    }
                    current_repo = Some((repo_name, deliver::PullRequestMetadata::default()));
                }
                DeliverOption::Name(title) => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.title, title, "name", repo)?;
                }
                DeliverOption::Body(body) => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.body, body, "body", repo)?;
                }
                DeliverOption::Draft => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.draft, true, "draft", repo)?;
                }
            }
        }

        if let Some((r, meta)) = current_repo {
            repo_overrides.push(deliver::RepoMetadataOverride {
                repo: r,
                metadata: meta,
            });
        }

        Ok((global_metadata, repo_overrides))
    }
}

impl From<FeatureDeliverArgs> for deliver::DeliverInput {
    fn from(args: FeatureDeliverArgs) -> Self {
        let FeatureDeliverArgs {
            feature,
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
            only,
        } = args;

        Self {
            feature: feature.unwrap_or_default(),
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
            only,
        }
    }
}
