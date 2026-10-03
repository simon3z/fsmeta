%define cargo_bin fsmeta

Name:           fsmeta
Version:        0.1.0
Release:        %{release}
Summary:        Filesystem metadata dump and diff tool
License:        Apache-2.0 AND Apache-2.0 WITH LLVM-exception AND LGPL-2.1-or-later AND MIT AND Unicode-3.0
URL:            https://github.com/simon3z/fsmeta
Source0:        %{name}-%{version}.tar.xz
Source1:        %{name}-%{version}-vendor.tar.xz

BuildRequires:  cargo-rpm-macros
BuildRequires:  rust
BuildRequires:  rust-packaging

%description
fsmeta dumps filesystem metadata (mode, uid/gid, timestamps, xattrs,
lsattr flags, SHA1 checksums) into a portable, versioned binary
snapshot and diffs two snapshots or a snapshot against a live
filesystem. Linux-only (kernel >= 4.19).

%prep
%autosetup -a 1
%cargo_prep -v vendor
%cargo_vendor_manifest

%build
%cargo_build

%install
export PATH="%{buildroot}%{_bindir}:$PATH"
%cargo_install

%files
%license LICENSE
%doc README.md FORMAT.md
%{_bindir}/%{name}

%changelog
* Sat Oct 03 2026 Federico Simoncelli <federico.simoncelli@gmail.com> - 0.1.0-1
- Initial package
