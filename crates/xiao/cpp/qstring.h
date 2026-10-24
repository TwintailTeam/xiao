#pragma once

#include <QtCore/QString>

#include "rust/cxx.h"

namespace rust {

template <>
struct IsRelocatable<QString> : ::std::true_type {};

}
