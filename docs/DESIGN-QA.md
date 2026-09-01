# CodeCaddie design QA

Run this checklist against an installed development build before a release.
Do not commit screenshots, local paths, generated audit transcripts, or fixed
test counts. Attach run-specific evidence to the release or pull request.

## Required flows

1. Attach a repository and validate the selected commit.
2. Enter product context, generate goals, and confirm the set contains every
   material business, architecture, and operations priority without a fixed
   quota per category.
3. Edit, add, reorder, delete, and restore goals. Confirm generated priorities
   and semantic IDs survive reordering and regeneration.
4. Run analysis with more than twelve goals. Confirm progress remains bounded,
   one failed batch leaves explicit unverified results, and an all-batch failure
   reports an error.
5. Inspect the report, evidence coordinates, historical N/A states, and Word
   export. Confirm no source excerpt or absolute repository path appears.
6. Cancel generation and analysis, retry both, relaunch the app, and confirm
   staged request files and partial operations are cleaned up.
7. Exercise keyboard navigation, screen-reader labels, reduced motion, high
   contrast, narrow window sizing, empty states, errors, and confirmation flows.

## Release evidence

Record the commit, package checksum, OS, architecture, provider CLI/version,
test command output, accessibility checks, and reviewer score in the release
record. A release candidate passes when every required flow works and the
current automated gates pass from a clean checkout.
