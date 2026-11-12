# Síťová umístění a budoucí filesystem plugin Network

Datum: 5. listopadu 2026, doplněno 9. listopadu. Stav: implementace discovery
pluginu a panelové integrace probíhá, platformní adaptéry mají samostatné testy.
Kvalifikace skutečného SMB sdílení a macOS zatím není dokončená.
Platí [společný plugin kontrakt](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).

## Rozsah a hranice

`midnight-filesystem-network` má vlastní Cargo projekt, build, testy a CI v jednom Git
repozitáři s aplikací; sdílí pouze veřejné
API/SDK a nesmí upravovat app sources. Balíček je v
`filesystems/network.windows/` nebo `filesystems/network.linux/`;
Windows/Linux a později macOS mají oddělené adaptery a testy.

Kořen Network nabídne objevená zařízení a uložené/ručně zadané cíle. Discovery
není úplný inventář sítě; prázdný výsledek nedokazuje nepřítomnost zařízení.
Ruční zadání funguje nezávisle na discovery a jeho oprávněních.

Zařízení, služba a share jsou různé uzly. Zařízení bez podporované služby není
filesystem. Plugin deklaruje protokoly; nepodporovaný vrátí Unsupported.
Discovery uzly neposkytují čtení obsahu ani zápis. Po výslovném otevření share
OS dodá skutečnou nativní cestu; úspěšné otevření této cesty přepne panel na
nativní filesystem a jeho existující operace včetně ověřovaného kopírování.

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

## Doplnění implementace 9. listopadu

- Veřejné ABI 1.2 přidává `Activate` (17), `Refresh` (18) a `ResolveText` (19).
  Stávající čísla operací a C layouty zůstávají zachované. Síťový balíček má
  `kind = "filesystem-network"`, opaque adresy a výhradně discovery capabilities.
- Windows používá oddělený Unicode MPR adaptér (`RESOURCE_GLOBALNET` bez rodiče
  pro rychlý výpis providerů v kořeni, se zachovanou identitou pro vybraný kontejner).
  Kořen nečeká na pomalé hledání okolních zařízení přes `RESOURCE_CONTEXT`.
  Linux používá oddělený GIO/GVfs
  adaptér a přesné `standard::target-uri`; skutečnou nativní cestu musí dodat OS.
  Bez GVfs/FUSE nelze virtuální URI použít jako cestu do nativního filesystemu.
- Explicitní otevření může inicializovat GIO namespace. Výpis, obnovení uložené
  adresy, hover, ikony ani metadata nepřipojují share a nevyvolávají přihlášení.
  Interaktivní přihlášení Linux adaptéru je v první etapě odmítnuté typovaným
  `AuthenticationRequired`; použít lze existující OS relaci. Aplikace hesla neukládá.
- Host si první stránku vyžádá s jedinou položkou, aby první zařízení nečekalo
  na pomalý další výsledek. Katalog posílá další stránky po nejvýše 128 položkách.
  Do 30sekundové cache vstupuje
  pouze dokončený výpis. Již publikované stránky zůstávají použitelné, i když další
  stránka selže. Refresh zneplatní starou generaci cache i rozpracované kurzory.
- Sdílené limity napříč instancemi pluginu: dvě aktivní OS operace/kurzory,
  64 mountů, 4096 vydaných uzlů a 16 MiB účtované paměti katalogu. Zrušené, ale
  dosud blokující OS volání si ponechá svůj slot až do skutečného návratu.
  Desetisekundový deadline je kooperativní, nikoli násilné ukončení OS funkce.
- Panelové položky nesou `NetworkNode`, nemají nativní cestu a nevstupují do
  mapy native-operation targets. Copy/move/delete/rename/checksum/size/context-menu
  zůstávají nedostupné, dokud není úspěšně otevřené konkrétní sdílení.
- Enter/dvojklik, parent/back, historie, řazení a druhý panel používají typované
  provider adresy. Neúspěšné předání share nezkouší jiný share ani systémový disk.
  Síťové karty se ukládají ve schématu preferences 24; obnova pouze listuje a
  automaticky nepřipojuje. Hot Paths podporují skutečné připojené síťové cesty;
  samotné discovery uzly se zatím ukládají pouze v kartách.
- Společná horní lišta obsahuje tlačítko Network Neighborhood se systémovou síťovou
  ikonou, případně existující vektorovou náhradou. Otevírá Network v aktivním panelu,
  má tooltip a bez dostupného pluginu je neaktivní. Nemění chování F11/F12 pro
  systémové připojení a odpojení síťových disků.
- Systémové služby nemusí objevit všechna zařízení. Ručně zadané UNC/SMB cíle
  nepotřebují úspěšnou předchozí discovery. Automatické subnet/port skeny nejsou
  součástí implementace. macOS později dostane vlastní Bonjour/OS adaptér.

Zkontrolované reference Salamanderu v `src/plugins/nethood/`: `nethoodfs.cpp:147–168`
rozlišuje server a share a při otevření share přechází na filesystem cestu;
`nethoodfs2.cpp:680–704` odděluje služby a filesystem ikonu;
`nethoodfs2.cpp:1178–1281` řeší čekající enumeraci a následnou obnovu panelu.
Přebírá se záměr rozhraní, nikoli GPL kód, prostředky, ANSI nebo pevné buffery.

## Upřesnění podle Salamanderu, 9. listopadu 2026

Znovu prověřeny `cache.cpp:1820–2019` (asynchronní enumerace kořene a jednotlivých
kontejnerů), `cache.cpp:2300–2398` (WNet a oddělená enumerace systémových sdílení),
`nethoodfs.cpp:105–181` (otevření zařízení versus předání share nativnímu panelu)
a `nethoodfs2.cpp:1190–1281` (částečné výsledky, pending/error a obnova panelu).
Zachovat tento uživatelský model a cache. Nepřebírat ANSI, MAX_PATH, ukončování
vláken ani čekání pod zámkem. Skrytá/systémová sdílení nejsou výchozí cíl průzkumu.

- **Windows:** vlastní adaptér používá Unicode WNet/MPR enumeraci, odděluje
  síťové kontejnery, servery a sdílení. Otevření share vrací hostu typovanou UNC
  cestu; přihlašování patří výslovné navigaci a systémové přihlašovací relaci.
- **Linux:** samostatný adaptér využije GIO/GVfs (`network://` a SMB backend)
  dostupný v desktopové relaci. Chybějící GVfs backend se hlásí jako nedostupná
  služba. URI se nikdy nevydává za POSIX cestu: předání native provideru vyžaduje
  skutečnou cestu poskytnutou OS. Nepřipojený cíl musí mít výslovný stav vyžadující
  připojení; listing nesmí sám spouštět mount ani dialog s heslem.
- **macOS později:** vlastní adaptér nad Bonjour a nativními službami pro síťové
  svazky, sdílející pouze protokol a platformně neutrální cache. Žádný Linux
  adaptér přeložený pod jiným `cfg`, žádné tvrzení o kvalifikované macOS podpoře.

Jde o dynamicky načítaný balíček `plugins/midnight-filesystem-network`, s vlastními
testy a závislostmi jen na veřejném filesystem API/SDK a bez přístupu ke zdrojům
aplikace. Platformní implementace a FFI jsou v oddělených souborech. Balíček se
instaluje do `filesystems/network.windows/` či `filesystems/network.linux/`;
jeho odebrání skryje discovery, nikoli
funkčnost již známé nativní UNC/mount cesty.

Veřejný kontrakt musí rozlišit pouhé rozlišení uložené adresy/listing od
**Activate** vyvolaného Enter/dvojklikem. Typovaný výsledek aktivace předává
nativní adresu až po explicitní volbě share; host nesmí tuto operaci volat při
restore, hoveru nebo tvorbě ikon. Přidaná operace je capability-gated, starší
native pluginy zůstanou kompatibilní a nepodporované operace explicitně odmítnou.
Uzly Network nejsou kandidáty pro copy/move/delete/rename/checksums/size scan;
tyto operace lze získat teprve po úspěšném přechodu do skutečného filesystemu.

První rozpočty: nejvýše 2 souběžné discovery požadavky pro celou aplikaci,
nejvýše 128 uzlů na odpověď, 4096 uzlů/16 MiB v cache, TTL 30 sekund. Každý
požadavek má generační identitu a 10sekundový deadline. Opožděná odpověď nesmí
obnovit starý panel. U nativního volání bez skutečného přerušení zůstane slot
obsazen do návratu; timeout nesmí vytvořit další neomezená vlákna. Ruční cíl,
již známá sdílení a druhý panel musí zůstat dostupné i při blokované enumeraci.
Testy nesmí bez výslovného spuštění provádět discovery v reálné firemní síti.

Primární reference: [WNetOpenEnumW](https://learn.microsoft.com/en-us/windows/win32/api/winnetwk/nf-winnetwk-wnetopenenumw),
[GIO FileEnumerator](https://docs.gtk.org/gio/class.FileEnumerator.html),
[Apple Bonjour](https://developer.apple.com/bonjour/).
