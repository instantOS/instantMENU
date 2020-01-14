# instantmenu - menu for instantOS
# See LICENSE file for copyright and license details.

include config.mk

SRC = drw.c instantmenu.c stest.c util.c
OBJ = $(SRC:.c=.o)

all: options instantmenu stest

options:
	@echo instantmenu build options:
	@echo "CFLAGS   = $(CFLAGS)"
	@echo "LDFLAGS  = $(LDFLAGS)"
	@echo "CC       = $(CC)"

.c.o:
	$(CC) -c $(CFLAGS) $<

config.h:
	cp config.def.h $@

$(OBJ): arg.h config.h config.mk drw.h

instantmenu: instantmenu.o drw.o util.o
	$(CC) -o $@ instantmenu.o drw.o util.o $(LDFLAGS)

stest: stest.o
	$(CC) -o $@ stest.o $(LDFLAGS)

clean:
	rm -f instantmenu stest $(OBJ) instantmenu-$(VERSION).tar.gz config.h instantmenu

dist: clean
	mkdir -p instantmenu-$(VERSION)
	cp LICENSE Makefile README arg.h config.def.h config.mk instantmenu.1\
		drw.h util.h instantmenu_path instantmenu_run stest.1 $(SRC)\
		instantmenu-$(VERSION)
	tar -cf instantmenu-$(VERSION).tar instantmenu-$(VERSION)
	gzip instantmenu-$(VERSION).tar
	rm -rf instantmenu-$(VERSION)

install: all
	mkdir -p $(DESTDIR)$(PREFIX)/bin
	cp -f instantmenu instantmenu_path instantmenu_run stest $(DESTDIR)$(PREFIX)/bin
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu_path
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu_run
	chmod 755 $(DESTDIR)$(PREFIX)/bin/stest
	mkdir -p $(DESTDIR)$(MANPREFIX)/man1
	sed "s/VERSION/$(VERSION)/g" < instantmenu.1 > $(DESTDIR)$(MANPREFIX)/man1/instantmenu.1
	sed "s/VERSION/$(VERSION)/g" < stest.1 > $(DESTDIR)$(MANPREFIX)/man1/stest.1
	chmod 644 $(DESTDIR)$(MANPREFIX)/man1/instantmenu.1
	chmod 644 $(DESTDIR)$(MANPREFIX)/man1/stest.1

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/instantmenu\
		$(DESTDIR)$(PREFIX)/bin/instantmenu_path\
		$(DESTDIR)$(PREFIX)/bin/instantmenu_run\
		$(DESTDIR)$(PREFIX)/bin/stest\
		$(DESTDIR)$(MANPREFIX)/man1/instantmenu.1\
		$(DESTDIR)$(MANPREFIX)/man1/stest.1

.PHONY: all options clean dist install uninstall
