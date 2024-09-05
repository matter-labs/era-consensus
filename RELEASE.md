# Release process

## Automatic releases

We use [release-please](https://github.com/googleapis/release-please) to manage releases, as well
as a custom automation to publish releases on [crates.io](https://crates.io/).

Any pull request name must follow [conventional commits](https://www.conventionalcommits.org/en/v1.0.0/)
specification, and then, based on the PR titles, a release pull request will be created, which
will take care of changelog generation.

Important: only `fix` and `feat` labels will trigger a release PR creation. So, if a `chore` or `ci`
PR will be merged right after release, the PR will not be created (they _will_ be included into a release,
if a release PR exists, but they won't trigger PR creation or appear in the changelog). If you want to make
sure that the change will trigger a release PR, mark the PR as `fix` or `feat`.

By default, a patch version will be bumped. If you want to bump a minor version, mark the PR as breaking with
an exclamation point, e.g. `feat!` or `fix!`.

It is recommended that each PR has a component mentioned, e.g. `feat(component): Change added`.

Once release PR is merged, it will be published to `crates.io`, and a notification will be sent to Slack.

## Manual releases

> [!WARNING]  
> Manual releases are discouraged, and should only be used as a last resort measure.
> Discuss the manual release with the team beforehand and prepare a plan.
> It is very likely that manual release will interfere with `release-please` configuration,
> which will have to be fixed manually as well.
>
> Additionally, if the release was created, but wasn't published, you will only need a subset
> of the actions listed below (e.g. if the it failed due to a transient error, you just need to
> publish code without creating any tags; but if the release can't be published, it's better to
> remove it, fix the issue, and try releasing again via automation).

> [!CAUTION]
> Never release code that does not correspond to any tag.

If you want to release the packages on crates.io manually, follow this process:

1. Install `cargo workspaces`: `cargo install cargo-workspaces`
2. Create a new branch to prepare a release.
3. Change versions in the `Cargo.toml`:
  - `version` in `[workspace.package]`
  - `version` in `[workspace.dependencies]` for all the relevant crates.
4. Run `cargo build`. It must succeed.
5. Commit changes.
6. Run `cargo ws publish --dry-run`. Check the output. It might fail, but it might be OK.
  - `error: config value 'http.cainfo' is not set` can be ignored.
  - There might be warnings, this is OK.
  - There might be errors related to the version resolution, e.g. `failed to select a version`
    (in particular, for `zkevm_test_harness`). It's due to a bug in cargo workspaces.
    Check that the packages it complains about actually have the specified version, and if so,
    it's safe to proceed.
7. Create a PR named `crates.io: Release <version>`. Get a review and merge it.
8. From the main branch _after_ you merge it, run `cargo ws publish --publish-as-is --allow-dirty`.
  - The `--publish-as-is` argument skips the versioning step, which you already did before.
  - The `--allow-dirty` argument is required, because `cargo ws` temporarily removes dev-dependencies
    during publishing.
  - Important: if something fails and you have to do changes to the code, it's safe to run the same
    command again. `cargo ws` will skip already published packages.
9. If something goes wrong, see recommendations below.
10. If everything is OK, create a tag: `git tag v<version>`, e.g. `git tag v0.150.4`
11. `git push --tags`
12. Go to the Releases in the GitHUb, and create a release for published version.
13. Make sure that `release-please` works.
