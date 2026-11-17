# Shared by the scripts in bin/.

die() {
  echo "$1" >&2
  exit 1
}

# folder_arg DIR -> DIR if it is a directory, else stop.
folder_arg() {
  [ -d "$1" ] || die "not a folder: $1"
  echo "$1"
}
