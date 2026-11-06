# Síťová umístění a budoucí filesystem plugin Network

Datum: 5. listopadu 2026. Stav: návrh budoucího discovery pluginu. Současná
etapa upravuje označení známé síťové lokace v panelu; neimplementuje objevování
zařízení. Platí [společný plugin kontrakt](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).

## Rozsah a hranice

`midnight-fs-network` má vlastní Cargo projekt, build, testy a CI v jednom Git
repozitáři s aplikací; sdílí pouze veřejné
API/SDK a nesmí upravovat app sources. Balíček je v `filesystems/network/`;
Windows/Linux a později macOS mají oddělené adaptery a testy.

Kořen Network nabídne objevená zařízení a uložené/ručně zadané cíle. Discovery
není úplný inventář sítě; prázdný výsledek nedokazuje nepřítomnost zařízení.
Ruční zadání funguje nezávisle na discovery a jeho oprávněních.

Zařízení, služba a share jsou různé uzly. Zařízení bez podporované služby není
filesystem. Plugin deklaruje protokoly; nepodporovaný vrátí Unsupported.
První etapa je čtecí; zápis vyžaduje společný verified-copy kontrakt.

## Označení aktuální lokace v panelu

Backend vrátí typovaná metadata lokace: Local/Network/Unknown, provider/mount
identity a vhodný druh ikony. UI podle nich pro síťovou cestu zobrazí v zavřeném
drive dropdownu **Network** a síťovou ikonu, přednostně systémovou. Úplná skutečná
cesta zůstane v breadcrumbs a používané raw identitě; Network ji nenahrazuje.

Windows adapter rozpoznává UNC i mapovaný remote disk, Linux adapter skutečný
síťový mount podle OS/provider metadat; macOS později dodá vlastní adapter.
UI neodhaduje síť z prefixu, `/mnt` ani display textu. Klasifikace je asynchronní
bez připojování k cíli; stará generace nesmí přepsat novější lokaci. Unknown
zůstane Unknown.

Plugin přidá Network root. Bez něj zmizí discovery, ale známá UNC cesta či
připojený síťový svazek mohou dál fungovat přes native provider. Jejich Network
label neslibuje discovery. Uložené pluginové adresy zůstanou jako nedostupné;
nativní cesty se nepřepisují na pluginová URI.

## Backendový kontrakt a bezpečnost

- `ListRoots`, `ListChildren`, `ResolveLocation`, `Parent`, `Stat` a capabilities
  používají společné generační identity. Objevený název je nedůvěryhodný display
  text; identita obsahuje endpoint, protokol a scope, nikoli jen jméno zařízení.
- Discovery běží až při otevření Network/obnovení, přes dostupné OS služby nebo
  omezený dokumentovaný lokální discovery protokol. Žádné plošné IP/port skeny,
  rekurzivní zkoušení sdílení, automatické mounty ani dotazy na libovolné subnety.
  Seznam je dávkovaný, má TTL, rušení, deadline a omezený počet výsledků/cache.
- Samotné discovery, hover a ikony neotevírají sdílení ani nezkoušejí credentials.
  Přístup k endpointu vzniká až výslovnou navigací uživatele. AuthenticationRequired
  jde hostitelskou cestou; žádná automatická elevation, credential spraying nebo
  přenos hesla/session na jiný server při přesměrování. Dostupná OS relace se může
  použít pouze pro vybraný endpoint podle jejího skutečného scope.
- Host spravuje přihlašování; hesla nejsou v URI, historii, manifestu ani logu.
  Cancel ukončí požadavek; žádný tichý downgrade bezpečného protokolu.
- Přechod na nativně dostupnou share proběhne typovaným navigačním cílem přes
  host broker, ne přímým voláním native pluginu či Shell. Čistě virtuální transport
  používá vlastní mount a content leases. Parent/back zachová původní network
  kontext; schopnosti read/seek/watch a konzistence se nepředstírají.
- Síť má konzervativní sdílený I/O rozpočet. Ztráta připojení, nedostupná služba
  a odepřený přístup jsou odlišné stavy; UI a druhý panel zůstávají použitelné.

## Přejímka a reference

V pluginovém CI ověřit: objevené i ručně zadané cíle; nedostupné discovery;
duplicitní Unicode jména; zařízení bez filesystemu; nepodporovaný protokol;
částečné výsledky/TTL/cancel; scope credentials a redirect; žádný sken, mount,
autentizace ani otevření obsahu při listingu/hoveru. Integrační test používá
vydanou host binárku, nikoli úpravy app sources.

Host testy ověří label/ikonu pro UNC, remote drive a síťový mount, úplnou cestu,
stale klasifikaci, Unknown a odebraný plugin s funkční známou nativní síťovou
cestou. macOS podpora vyžaduje implementaci a kvalifikaci jeho adapteru.

Zkontrolované reference Salamanderu v `src/plugins/nethood/`: `nethoodfs.cpp:147–168`
rozlišuje server a share a při otevření share přechází na filesystem cestu;
`nethoodfs2.cpp:680–704` odděluje služby a filesystem ikonu;
`nethoodfs2.cpp:1178–1281` řeší čekající enumeraci a následnou obnovu panelu.
Přebírá se záměr rozhraní, nikoli GPL kód, prostředky, ANSI nebo pevné buffery.
