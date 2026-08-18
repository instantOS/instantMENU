# instantmenu - menu for instantOS
# See LICENSE file for copyright and license details.

include config.mk

SRC = drw.c instantmenu.c itest.c util.c
OBJ = $(SRC:.c=.o)

all: instantmenu itest

.c.o:
	$(CC) -g -c $(CFLAGS) $<

config.h:
	cp config.def.h $@

$(OBJ): arg.h config.h config.mk drw.h

instantmenu: instantmenu.o drw.o util.o
	$(CC) -g -o $@ instantmenu.o drw.o util.o $(LDFLAGS)

itest: itest.o
	$(CC) -g -o $@ itest.o $(LDFLAGS)

clean:
	rm -f instantmenu *.o *.out itest $(OBJ) instantmenu-$(VERSION).tar.gz config.h instantmenu

dist: clean
	mkdir -p instantmenu-$(VERSION)
	cp LICENSE Makefile README arg.h config.def.h config.mk instantmenu.1\
		drw.h util.h instantmenu_path instantmenu_run instantmenu_smartrun itest.1 $(SRC)\
		instantmenu-$(VERSION)
	tar -cf instantmenu-$(VERSION).tar instantmenu-$(VERSION)
	gzip instantmenu-$(VERSION).tar
	rm -rf instantmenu-$(VERSION)

install: all
	mkdir -p $(DESTDIR)$(PREFIX)/bin
	cp -f instantmenu instantmenu_path instantmenu_run instantmenu_smartrun itest $(DESTDIR)$(PREFIX)/bin
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu_path
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu_run
	chmod 755 $(DESTDIR)$(PREFIX)/bin/instantmenu_smartrun
	chmod 755 $(DESTDIR)$(PREFIX)/bin/itest
	mkdir -p $(DESTDIR)$(MANPREFIX)/man1
	sed "s/VERSION/$(VERSION)/g" < instantmenu.1 > $(DESTDIR)$(MANPREFIX)/man1/instantmenu.1
	sed "s/VERSION/$(VERSION)/g" < itest.1 > $(DESTDIR)$(MANPREFIX)/man1/itest.1
	chmod 644 $(DESTDIR)$(MANPREFIX)/man1/instantmenu.1
	chmod 644 $(DESTDIR)$(MANPREFIX)/man1/itest.1

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/instantmenu\
		$(DESTDIR)$(PREFIX)/bin/instantmenu_path\
		$(DESTDIR)$(PREFIX)/bin/instantmenu_run\
		$(DESTDIR)$(PREFIX)/bin/instantmenu_smartrun\
		$(DESTDIR)$(PREFIX)/bin/itest\
		$(DESTDIR)$(MANPREFIX)/man1/instantmenu.1\
		$(DESTDIR)$(MANPREFIX)/man1/itest.1

.PHONY: all clean dist install uninstall
