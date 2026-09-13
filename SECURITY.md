# Security policy

## Supported versions

Corsair Control is experimental pre-release software and has not received an
independent security audit. Security fixes target the latest release and the
`main` branch. Older releases do not receive backported security fixes; upgrade
to the latest release when a fix is published.

## Reporting a vulnerability

Please report suspected vulnerabilities privately using
[GitHub's Report a vulnerability form](https://github.com/plasticparticle/corsair-linux-ui/security/advisories/new).
Private vulnerability reporting is enabled for this repository.

Do not disclose exploit details in public issues, discussions, or pull requests
before coordinating disclosure with the maintainer. Use public issues for
ordinary bugs that do not have security implications.

Include as much of the following as possible:

- The affected release or commit and installation method.
- Your Linux distribution, kernel version, and relevant device model, USB IDs,
  firmware version, and connection type.
- Steps to reproduce, a minimal proof of concept, and expected versus actual
  behavior.
- The potential impact and any required permissions or user interaction.
- Relevant logs or configuration with credentials, personal data, and recorded
  keystrokes removed.

Areas of particular interest include device permissions and udev rules, raw
input capture and virtual input injection, Tauri command access, profile parsing
and storage, hardware communication, and installation or release tooling.

Test only on systems and devices you own or have permission to test. Avoid tests
that expose other people's input or data or risk damaging hardware.

## Response and disclosure

Reports are handled on a best-effort basis; this community project does not
guarantee a response or remediation deadline. The maintainer will use the private
report to discuss findings, possible mitigations, and a coordinated disclosure
date. Please follow up in that report if you have additional information.

Confirmed vulnerabilities may be published as GitHub security advisories with
affected versions, available fixes, and mitigations. Reporter credit can be
included with the reporter's consent.

This policy covers this independent project, not Corsair's products or services.
