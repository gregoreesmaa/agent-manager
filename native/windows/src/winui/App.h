// Implementation headers for the XAML classes (issue #64).
//
// The codegen step emits starter stubs with these names under
// `Generated Files\sources`, but those are starting points, not the
// implementation. The real types live in the accompanying .xaml.h,
// so forward to it: this also satisfies module.g.cpp, which includes
// "App.h" by the stub's name.
#pragma once

#include "App.xaml.h"
