#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

function parseArgs(args) {
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error("usage: generate-wix.mjs --payload DIR --version X.Y.Z --build N --output FILE");
    }
    values[name.slice(2)] = value;
  }
  for (const required of ["payload", "version", "build", "output"]) {
    if (!values[required]) throw new Error(`--${required} is required`);
  }
  values.msiVersion = windowsInstallerVersion(values.version, values.build);
  return values;
}

export function windowsInstallerVersion(version, releaseBuild) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-rc\.(\d+))?$/.exec(version);
  if (!match) throw new Error("version must be X.Y.Z or X.Y.Z-rc.N");
  const [, majorText, minorText] = match;
  const major = Number(majorText);
  const minor = Number(minorText);
  const build = Number(releaseBuild);
  if (
    major > 255 ||
    minor > 255 ||
    !Number.isSafeInteger(build) ||
    build < 1 ||
    build > 65_535
  ) {
    throw new Error("version cannot be represented as a monotonic MSI product version");
  }
  return `${major}.${minor}.${build}`;
}

function id(prefix, value) {
  return `${prefix}_${createHash("sha256").update(value).digest("hex").slice(0, 24)}`;
}

function guid(value) {
  const bytes = Buffer.from(createHash("sha256").update(value).digest().subarray(0, 16));
  bytes[6] = (bytes[6] & 0x0f) | 0x50;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes.toString("hex").toUpperCase();
  return `{${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}}`;
}

function xml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

async function walk(root, relative = "") {
  const entries = await readdir(path.join(root, relative), { withFileTypes: true });
  const directories = [];
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    if (entry.name.startsWith(".")) continue;
    const next = path.posix.join(relative.replaceAll(path.sep, "/"), entry.name);
    if (entry.isDirectory()) {
      const nested = await walk(root, next);
      directories.push({ name: entry.name, relative: next, ...nested });
    } else if (entry.isFile()) {
      files.push({ name: entry.name, relative: next });
    }
  }
  return { directories, files };
}

function renderDirectory(tree, relative = "", indent = "          ") {
  const directoryId = relative ? id("Dir", relative) : "INSTALLFOLDER";
  const components = tree.files
    .map((file) => {
      const componentId = id("Cmp", file.relative);
      const fileId = id("File", file.relative);
      const registryName = createHash("sha256").update(file.relative).digest("hex");
      const source = `!(bindpath.Payload)\\${file.relative.replaceAll("/", "\\")}`;
      return `${indent}<Component Id="${componentId}" Guid="${guid(`codecaddie:${file.relative}`)}" Directory="${directoryId}">
${indent}  <File Id="${fileId}" Source="${xml(source)}" />
${indent}  <RegistryValue Root="HKCU" Key="Software\\CodeCaddie\\InstallerComponents" Name="${registryName}" Type="integer" Value="1" KeyPath="yes" />
${indent}</Component>`;
    })
    .join("\n");
  const directories = tree.directories
    .map((directory) => {
      const directoryMarkup = `${indent}<Directory Id="${id("Dir", directory.relative)}" Name="${xml(directory.name)}">\n${renderDirectory(directory, directory.relative, `${indent}  `)}\n${indent}</Directory>`;
      return directoryMarkup;
    })
    .join("\n");
  return [components, directories].filter(Boolean).join("\n");
}

function collectComponentIds(tree) {
  return [
    ...tree.files.map((file) => id("Cmp", file.relative)),
    ...tree.directories.flatMap(collectComponentIds),
  ];
}

export async function generateWix(options) {
  const payload = path.resolve(options.payload);
  const output = path.resolve(options.output);
  const tree = await walk(payload);
  const componentRefs = collectComponentIds(tree)
    .map((componentId) => `      <ComponentRef Id="${componentId}" />`)
    .join("\n");
  const markup = `<?xml version="1.0" encoding="utf-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package
    Name="CodeCaddie"
    Manufacturer="Tailored AI Solutions"
    Version="${options.msiVersion}"
    UpgradeCode="{A7D45C1F-04E7-4A0E-9D23-6D9B5C5E3F10}"
    Scope="perUser"
    InstallerVersion="500">
    <MajorUpgrade DowngradeErrorMessage="A newer version of CodeCaddie is already installed." />
    <MediaTemplate EmbedCab="yes" CompressionLevel="high" />
    <SummaryInformation Description="CodeCaddie local-first desktop application" />

    <StandardDirectory Id="LocalAppDataFolder">
      <Directory Id="ProgramsFolder" Name="Programs">
        <Directory Id="INSTALLFOLDER" Name="CodeCaddie">
${renderDirectory(tree)}
        </Directory>
      </Directory>
    </StandardDirectory>

    <StandardDirectory Id="ProgramMenuFolder">
      <Directory Id="ApplicationProgramsFolder" Name="CodeCaddie">
        <Component Id="StartMenuShortcut" Guid="{0C68F394-B91A-4A85-BE41-4EC86D15F161}">
          <Shortcut Id="ApplicationStartMenuShortcut" Name="CodeCaddie" Target="[INSTALLFOLDER]bin\\codecaddie.exe" WorkingDirectory="INSTALLFOLDER" />
          <RemoveFolder Id="ApplicationProgramsFolder" On="uninstall" />
          <RegistryValue Root="HKCU" Key="Software\\CodeCaddie" Name="StartMenuShortcut" Type="integer" Value="1" KeyPath="yes" />
        </Component>
      </Directory>
    </StandardDirectory>

    <Component Id="UserAssociations" Guid="{E59C3DB8-5AA2-4DA2-A5D5-811B47CFC24F}" Directory="INSTALLFOLDER">
      <RegistryValue Root="HKCU" Key="Software\\Classes\\codecaddie" Value="URL:CodeCaddie Protocol" Type="string" />
      <RegistryValue Root="HKCU" Key="Software\\Classes\\codecaddie" Name="URL Protocol" Value="" Type="string" />
      <RegistryValue Root="HKCU" Key="Software\\Classes\\codecaddie\\shell\\open\\command" Value="&quot;[INSTALLFOLDER]bin\\codecaddie.exe&quot; &quot;%1&quot;" Type="string" />
      <RegistryValue Root="HKCU" Key="Software\\Classes\\.codecaddie" Value="CodeCaddie.Workspace" Type="string" />
      <RegistryValue Root="HKCU" Key="Software\\Classes\\CodeCaddie.Workspace" Value="CodeCaddie Workspace" Type="string" />
      <RegistryValue Root="HKCU" Key="Software\\Classes\\CodeCaddie.Workspace\\shell\\open\\command" Value="&quot;[INSTALLFOLDER]bin\\codecaddie.exe&quot; &quot;%1&quot;" Type="string" KeyPath="yes" />
    </Component>

    <Feature Id="ProductFeature" Title="CodeCaddie" Level="1">
${componentRefs}
      <ComponentRef Id="StartMenuShortcut" />
      <ComponentRef Id="UserAssociations" />
    </Feature>
  </Package>
</Wix>
`;
  await writeFile(output, markup, "utf8");
  return output;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const output = await generateWix(parseArgs(process.argv.slice(2)));
  console.log(output);
}
