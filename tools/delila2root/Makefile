CXX      = $(shell root-config --cxx)
CXXFLAGS = -O3 -Wall -Wextra $(shell root-config --cflags)
LDFLAGS  = $(shell root-config --libs)

# Optional: uncomment for platform-specific optimization
# CXXFLAGS += -march=native -flto
# LDFLAGS  += -flto

PREFIX   = /usr/local
TARGET   = delila2root
SRC      = delila2root.cpp

.PHONY: all clean install uninstall

all: $(TARGET)

$(TARGET): $(SRC)
	$(CXX) $(CXXFLAGS) -o $@ $< $(LDFLAGS)

install: $(TARGET)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 755 $(TARGET) $(DESTDIR)$(PREFIX)/bin/

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(TARGET)

clean:
	rm -f $(TARGET)
